//! A picture drawn on the graphics card, under the window's own (software) controls.
//!
//! The window's surface carries the toolkit's pixels; the picture lives on a surface of its own, a subsurface placed below,
//! which wgpu presents to. Zooming and panning then cost one textured quad per frame instead of a bilinear pass over every
//! pixel of the window on the processor. The picture is uploaded once (with its mips) and kept for the next time it is shown.
//!
//! Live frames (a camera, a video) use the same surface: [`GpuLayer::render_live`] keeps one texture per format and size,
//! writes each new frame into it and converts YUV to RGB in the shader, so nothing is converted on the processor.
//!
//! The integrated card is used: the picture is only sampled, and the dedicated one would be woken for nothing.
//! `CHEBRIUM_NO_GPU=1` keeps the picture on the processor.

use crate::live::{Frame, LiveLayer, PixelFormat};
use crate::picture::{PictureLayer, Pic};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use wgpu::util::DeviceExt;

const SHADER: &str = "
struct Params {
    rect: vec4<f32>,
    uv: vec4<f32>,
    view: vec2<f32>,
    alpha: f32,
    lod: f32,
    nearest: u32,
    mode: u32,
    flags: u32,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var smooth_s: sampler;
@group(0) @binding(3) var crisp_s: sampler;
@group(0) @binding(4) var tex2: texture_2d<f32>;

struct V {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> V {
    var corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    let c = corners[i];
    let px = p.rect.xy + c * p.rect.zw;
    var o: V;
    o.pos = vec4(px.x / p.view.x * 2.0 - 1.0, 1.0 - px.y / p.view.y * 2.0, 0.0, 1.0);
    o.uv = mix(p.uv.xy, p.uv.zw, c);
    return o;
}

// flags: bit 0 limited (video) range, bit 1 BT.709 instead of BT.601; mirrors Frame::to_pic on the processor
fn yuv(y: f32, u: f32, v: f32) -> vec3<f32> {
    var l = y;
    var cb = u - 128.0 / 255.0;
    var cr = v - 128.0 / 255.0;
    if (p.flags & 1u) != 0u {
        l = (y - 16.0 / 255.0) * 255.0 / 219.0;
        cb = cb * 255.0 / 224.0;
        cr = cr * 255.0 / 224.0;
    }
    if (p.flags & 2u) != 0u {
        return clamp(vec3(l + 1.5748 * cr, l - 0.1873 * cb - 0.4681 * cr, l + 1.8556 * cb), vec3(0.0), vec3(1.0));
    }
    return clamp(vec3(l + 1.402 * cr, l - 0.344136 * cb - 0.714136 * cr, l + 1.772 * cb), vec3(0.0), vec3(1.0));
}

@fragment
fn fs(v: V) -> @location(0) vec4<f32> {
    var c: vec4<f32>;
    if p.mode == 1u {
        // NV12: luma plane in tex, interleaved chroma in tex2
        let y = textureSampleLevel(tex, smooth_s, v.uv, 0.0).r;
        let uv = textureSampleLevel(tex2, smooth_s, v.uv, 0.0).rg;
        c = vec4(yuv(y, uv.x, uv.y), 1.0);
    } else if p.mode == 2u {
        // YUYV: one texel is Y0 U Y1 V for two pixels, read exactly
        let d = textureDimensions(tex);
        let ix = clamp(i32(floor(v.uv.x * f32(d.x * 2u))), 0, i32(d.x * 2u) - 1);
        let iy = clamp(i32(floor(v.uv.y * f32(d.y))), 0, i32(d.y) - 1);
        let t = textureLoad(tex, vec2<i32>(ix / 2, iy), 0);
        c = vec4(yuv(select(t.r, t.b, (ix % 2) == 1), t.g, t.a), 1.0);
    } else if p.nearest != 0u {
        c = textureSampleLevel(tex, crisp_s, v.uv, 0.0);
    } else {
        c = textureSampleLevel(tex, smooth_s, v.uv, p.lod);
    }
    // premultiplied, thinned out by the picture's own opacity
    return c * p.alpha;
}
";

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    rect: [f32; 4],
    uv: [f32; 4],
    view: [f32; 2],
    alpha: f32,
    lod: f32,
    nearest: u32,
    /// 0 a picture, 1 NV12, 2 YUYV
    mode: u32,
    flags: u32,
    _pad: u32,
}

/// What was drawn last: when nothing differs, the frame is not drawn again.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    live: bool,
    mirror: bool,
    pic: u64,
    size: [u32; 2],
    rect: [f32; 4],
    src: [f32; 4],
    alpha: f32,
    ground: [f32; 3],
}

/// The textures a live frame is written into: the same ones for every frame of a format and size.
struct LiveTex {
    format: PixelFormat,
    size: [u32; 2],
    main: wgpu::Texture,
    main_view: wgpu::TextureView,
    chroma: Option<wgpu::Texture>,
    chroma_view: Option<wgpu::TextureView>,
    /// id of the frame whose bytes are in the textures
    uploaded: u64,
}

struct Uploaded {
    id: u64,
    view: wgpu::TextureView,
    mips: u32,
    /// frame counter of the last use, to drop the least recently shown
    used: u64,
}

pub struct GpuLayer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    configured: [u32; 2],
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    smooth: wgpu::Sampler,
    crisp: wgpu::Sampler,
    textures: Vec<Uploaded>,
    /// the live frame's textures, updated in place
    live: Option<LiveTex>,
    /// bound as the second plane when there is none
    dummy: wgpu::TextureView,
    max_side: u32,
    frame: u64,
    drawn: Option<Drawn>,
}

/// How many pictures stay on the card: the one shown and its neighbours.
const KEEP: usize = 3;

impl GpuLayer {
    /// Open the card for a surface. `display` and `surface` are the `wl_display` and `wl_surface` pointers.
    pub fn new(display: NonNull<c_void>, surface: NonNull<c_void>) -> Result<GpuLayer, String> {
        if std::env::var_os("CHEBRIUM_NO_GPU").is_some() {
            return Err("CHEBRIUM_NO_GPU is set".into());
        }
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: wgpu::rwh::RawDisplayHandle::Wayland(wgpu::rwh::WaylandDisplayHandle::new(display)),
            raw_window_handle: wgpu::rwh::RawWindowHandle::Wayland(wgpu::rwh::WaylandWindowHandle::new(surface)),
        };
        // the surface lives as long as the layer, and the layer is dropped before the wl_surface it was made for
        let surf = unsafe { instance.create_surface_unsafe(target) }.map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surf),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let max_side = limits.max_texture_dimension_2d;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("chebrium picture"), required_limits: limits, ..Default::default() })).map_err(|e| e.to_string())?;

        let caps = surf.get_capabilities(&adapter);
        // the picture's bytes go through as they are, as on the processor: no sRGB conversion
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).ok_or("no plain surface format")?;
        let alpha_mode = caps.alpha_modes.iter().copied().find(|m| *m == wgpu::CompositeAlphaMode::Opaque).unwrap_or(caps.alpha_modes[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: 1,
            height: 1,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("picture"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("picture"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("picture"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let sampler = |filter| device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: filter, min_filter: filter, mipmap_filter: filter, ..Default::default() });
        let (smooth, crisp) = (sampler(wgpu::FilterMode::Linear), sampler(wgpu::FilterMode::Nearest));
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no plane"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(GpuLayer { surface: surf, device, queue, config, configured: [0, 0], pipeline, layout, smooth, crisp, textures: Vec::new(), live: None, dummy, max_side, frame: 0, drawn: None })
    }

    /// Put the picture on the card (once per picture); the index of its texture.
    fn upload(&mut self, pic: &Arc<Pic>) -> Result<usize, String> {
        self.frame += 1;
        if let Some(i) = self.textures.iter().position(|t| t.id == pic.id) {
            self.textures[i].used = self.frame;
            return Ok(i);
        }
        if pic.w > self.max_side || pic.h > self.max_side {
            return Err(format!("a {}x{} picture is larger than the card takes ({})", pic.w, pic.h, self.max_side));
        }
        let mips = pic.level_count() as u32;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("picture"),
            size: wgpu::Extent3d { width: pic.w, height: pic.h, depth_or_array_layers: 1 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for i in 0..pic.level_count() {
            let (data, w, h) = pic.level(i);
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        if self.textures.len() >= KEEP {
            let oldest = (0..self.textures.len()).min_by_key(|&i| self.textures[i].used).unwrap_or(0);
            self.textures.swap_remove(oldest);
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.textures.push(Uploaded { id: pic.id, view, mips, used: self.frame });
        Ok(self.textures.len() - 1)
    }

    /// Size the surface for a window of `size` device pixels.
    fn fit_surface(&mut self, size: [u32; 2]) {
        if self.configured != size {
            self.config.width = size[0].max(1);
            self.config.height = size[1].max(1);
            self.surface.configure(&self.device, &self.config);
            self.configured = size;
        }
    }

    /// Draw the layer for a window of `size` device pixels at `scale` device pixels per logical one. Nothing is drawn (and
    /// nothing presented) when it looks as it did last time.
    pub fn render(&mut self, size: [u32; 2], scale: f32, l: &PictureLayer) -> Result<(), String> {
        let rect = [l.dest[0] * scale, l.dest[1] * scale, l.dest[2] * scale, l.dest[3] * scale];
        let now = Drawn { live: false, mirror: false, pic: l.pic.id, size, rect, src: l.src, alpha: l.alpha, ground: l.ground };
        if self.drawn == Some(now) {
            return Ok(());
        }
        let index = self.upload(&l.pic)?;
        self.fit_surface(size);
        let tex = &self.textures[index];
        // pixels on the screen per picture pixel; a smaller mip once the picture is shown smaller than it is
        let k = (l.dest[2] / l.src[2].max(1e-6) * scale).min(l.dest[3] / l.src[3].max(1e-6) * scale);
        let lod = if k < 1.0 { (1.0 / k.max(1e-6)).log2().clamp(0.0, (tex.mips - 1) as f32) } else { 0.0 };
        let (pw, ph) = (l.pic.w as f32, l.pic.h as f32);
        let params = Params {
            rect,
            uv: [l.src[0] / pw, l.src[1] / ph, (l.src[0] + l.src[2]) / pw, (l.src[1] + l.src[3]) / ph],
            view: [size[0] as f32, size[1] as f32],
            alpha: l.alpha.clamp(0.0, 1.0),
            lod,
            // magnified a lot, the pixels stay crisp
            nearest: (k > 3.0) as u32,
            mode: 0,
            flags: 0,
            _pad: 0,
        };
        self.paint(params, &tex.view, &self.dummy, l.ground)?;
        self.drawn = Some(now);
        Ok(())
    }

    /// Make room for a live frame: the textures of its format and size (made once, kept for the following frames).
    fn live_textures(&mut self, f: &Frame) -> Result<(), String> {
        if f.w > self.max_side || f.h > self.max_side {
            return Err(format!("a {}x{} frame is larger than the card takes ({})", f.w, f.h, self.max_side));
        }
        if self.live.as_ref().is_some_and(|t| t.format == f.format && t.size == [f.w, f.h]) {
            return Ok(());
        }
        let make = |label, w, h, format| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        use wgpu::TextureFormat as F;
        let (main, chroma) = match f.format {
            PixelFormat::Bgra => (make("live frame", f.w, f.h, F::Bgra8Unorm), None),
            PixelFormat::Nv12 => (make("live luma", f.w, f.h, F::R8Unorm), Some(make("live chroma", f.w / 2, f.h / 2, F::Rg8Unorm))),
            PixelFormat::Yuyv => (make("live yuyv", f.w / 2, f.h, F::Rgba8Unorm), None),
        };
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        self.live = Some(LiveTex { format: f.format, size: [f.w, f.h], main_view: view(&main), chroma_view: chroma.as_ref().map(view), main, chroma, uploaded: 0 });
        self.drawn = None;
        Ok(())
    }

    /// Write a new frame's bytes into the live textures (nothing happens for the frame that is already there).
    fn live_upload(&mut self, f: &Frame) {
        let Some(t) = self.live.as_mut().filter(|t| t.uploaded != f.id) else { return };
        let (w, h) = (f.w, f.h);
        let write = |texture: &wgpu::Texture, data: &[u8], row: u32, width: u32, height: u32| {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
        };
        match f.format {
            PixelFormat::Bgra => write(&t.main, &f.data, w * 4, w, h),
            PixelFormat::Nv12 => {
                let (luma, chroma) = f.data.split_at((w * h) as usize);
                write(&t.main, luma, w, w, h);
                if let Some(c) = &t.chroma {
                    write(c, chroma, w, w / 2, h / 2);
                }
            }
            PixelFormat::Yuyv => write(&t.main, &f.data, w * 2, w / 2, h),
        }
        t.uploaded = f.id;
    }

    /// Draw a live frame, like [`GpuLayer::render`] draws a picture. The frame goes into textures that stay on the card;
    /// a frame already shown, in the same place, costs nothing.
    pub fn render_live(&mut self, size: [u32; 2], scale: f32, l: &LiveLayer) -> Result<(), String> {
        let f = &l.frame;
        let rect = [l.dest[0] * scale, l.dest[1] * scale, l.dest[2] * scale, l.dest[3] * scale];
        let now = Drawn { live: true, mirror: l.mirror, pic: f.id, size, rect, src: l.src, alpha: l.alpha, ground: l.ground };
        if self.drawn == Some(now) {
            return Ok(());
        }
        self.live_textures(f)?;
        self.live_upload(f);
        self.fit_surface(size);
        let Some(t) = &self.live else { return Err("no live textures".into()) };
        let (fw, fh) = (f.w as f32, f.h as f32);
        let (mut u0, mut u1) = (l.src[0] / fw, (l.src[0] + l.src[2]) / fw);
        if l.mirror {
            std::mem::swap(&mut u0, &mut u1);
        }
        let k = (l.dest[2] / l.src[2].max(1e-6) * scale).min(l.dest[3] / l.src[3].max(1e-6) * scale);
        let params = Params {
            rect,
            uv: [u0, l.src[1] / fh, u1, (l.src[1] + l.src[3]) / fh],
            view: [size[0] as f32, size[1] as f32],
            alpha: l.alpha.clamp(0.0, 1.0),
            lod: 0.0,
            nearest: (k > 3.0) as u32,
            mode: match f.format {
                PixelFormat::Bgra => 0,
                PixelFormat::Nv12 => 1,
                PixelFormat::Yuyv => 2,
            },
            flags: f.color.limited as u32 | (f.color.bt709 as u32) << 1,
            _pad: 0,
        };
        self.paint(params, &t.main_view, t.chroma_view.as_ref().unwrap_or(&self.dummy), l.ground)?;
        self.drawn = Some(now);
        Ok(())
    }

    /// One textured quad over the ground colour, presented.
    fn paint(&self, params: Params, main: &wgpu::TextureView, chroma: &wgpu::TextureView, ground: [f32; 3]) -> Result<(), String> {
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("picture"), contents: bytemuck::bytes_of(&params), usage: wgpu::BufferUsages::UNIFORM });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("picture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(main) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.smooth) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.crisp) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(chroma) },
            ],
        });

        use wgpu::SurfaceError::{Lost, Outdated};
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(Lost | Outdated) => {
                self.surface.configure(&self.device, &self.config);
                self.surface.get_current_texture().map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("picture") });
        {
            let g = ground;
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("picture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: g[0] as f64, g: g[1] as f64, b: g[2] as f64, a: 1.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..6, 0..1);
        }
        self.queue.submit([enc.finish()]);
        frame.present();
        Ok(())
    }

    /// The layer is hidden for now: the next `render` draws even if nothing differs.
    pub fn forget(&mut self) {
        self.drawn = None;
    }
}

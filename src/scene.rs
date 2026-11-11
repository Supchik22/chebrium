//! A program's own drawing on the graphics card: a 3D map, a game view, anything a picture cannot carry.
//!
//! The program implements [`Scene`] (it owns its pipelines and buffers, made on the first `render` for the device it is
//! given) and sets `Host::scene` each frame, only when `Host::gpu_layer` is true. The runtime draws the scene into a
//! texture of the window's size on the same card as the picture layer, then shows that texture under the window with the
//! glass panels over it, exactly like a picture. Nothing is drawn while `version` and the size stay the same.
//!
//! [`Offscreen`] draws a scene on a card of its own and reads the frame back as BGRA bytes for a `Pic`: for tests without a
//! window, and for programs that want the picture on the processor.

pub use wgpu;
use std::sync::{Arc, Mutex};

/// The texture format a scene draws into: plain bytes, no sRGB conversion (the shader writes display values).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The card a scene draws with. `id` changes when the device is a different one (the window lost its card and opened
/// another): everything made for the old one must be made again.
pub struct Gpu<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub id: u64,
}

pub trait Scene: Send {
    /// Draw a frame into `target` (`FORMAT`, `size` device pixels; what is in it is undefined, so clear it).
    fn render(&mut self, gpu: &Gpu<'_>, target: &wgpu::TextureView, size: [u32; 2]);
}

/// What `Host::scene` carries.
#[derive(Clone)]
pub struct SceneLayer {
    pub scene: Arc<Mutex<dyn Scene>>,
    /// Bump it whenever anything the scene shows changed. The same version at the same size is not drawn again.
    pub version: u64,
    /// where it goes, in logical window pixels
    pub dest: [f32; 4],
    /// the opaque ground behind it, which fills the whole window
    pub ground: [f32; 3],
}

impl std::fmt::Debug for SceneLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneLayer").field("version", &self.version).field("dest", &self.dest).finish()
    }
}

static DEVICE_IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A new number for a device just made.
pub fn next_device_id() -> u64 {
    DEVICE_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A card of its own, without a window.
pub struct Offscreen {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub id: u64,
    /// the adapter's name, for logs
    pub name: String,
}

impl Offscreen {
    /// Open a card: the integrated one when `low_power`, else the fastest.
    pub fn new(low_power: bool) -> Result<Offscreen, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let power_preference = if low_power { wgpu::PowerPreference::LowPower } else { wgpu::PowerPreference::HighPerformance };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference, compatible_surface: None, force_fallback_adapter: false })).map_err(|e| e.to_string())?;
        let name = adapter.get_info().name;
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("chebrium offscreen"), required_limits: limits, ..Default::default() })).map_err(|e| e.to_string())?;
        Ok(Offscreen { device, queue, id: next_device_id(), name })
    }

    pub fn gpu(&self) -> Gpu<'_> {
        Gpu { device: &self.device, queue: &self.queue, id: self.id }
    }

    /// Draw `scene` at `size` and read the frame back: premultiplied BGRA bytes, row after row, no padding.
    pub fn render(&self, scene: &mut dyn Scene, size: [u32; 2]) -> Result<Vec<u8>, String> {
        let [w, h] = [size[0].max(1), size[1].max(1)];
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen scene"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        scene.render(&self.gpu(), &view, [w, h]);
        read_back(&self.device, &self.queue, &texture, [w, h])
    }
}

/// Copy an RGBA8 texture to the processor as BGRA bytes.
pub fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, size: [u32; 2]) -> Result<Vec<u8>, String> {
    let [w, h] = size;
    let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("read back"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("read back") });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::Wait);
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let line = &data[y * row as usize..y * row as usize + w as usize * 4];
        for p in line.chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    drop(data);
    buffer.unmap();
    Ok(out)
}

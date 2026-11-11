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
use crate::record::GlassRect;
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
    // where the picture is drawn (px), the ground colour, a glass panel's [radius px, strength, dark, scale], the picture's
    // [highest mip level, screen px per texel], and the panel's [centre x, centre y, half width, half height] (px)
    pic: vec4<f32>,
    ground: vec4<f32>,
    glass: vec4<f32>,
    gfx: vec4<f32>,
    bx: vec4<f32>,
    // the theme's glass: [blur, refraction, bezel, dispersion], [rim, rim sharpness, saturation, -], tint, border
    look: vec4<f32>,
    look2: vec4<f32>,
    tint: vec4<f32>,
    border: vec4<f32>,
    // a correction (flags bit 2): [gain r, gain g, gain b, gamma], [black, white, sharpen, saturation]
    grade: vec4<f32>,
    grade2: vec4<f32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var smooth_s: sampler;
@group(0) @binding(3) var crisp_s: sampler;
@group(0) @binding(4) var tex2: texture_2d<f32>;
@group(0) @binding(5) var mask: texture_2d<f32>;

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


// ---- FSR 1 (AMD FidelityFX Super Resolution 1, MIT): EASU, an edge-adaptive upscale from 12 source taps, then RCAS, a
// contrast-adaptive sharpen over the result. Written for this shader from the published algorithm; one pass, so the
// sharpen reads the upscale at its four neighbours (5 x 12 taps).

fn src_size() -> vec2<f32> {
    let d = vec2<f32>(textureDimensions(tex));
    return select(d, vec2(d.x * 2.0, d.y), p.mode == 2u);
}

// the colour of one source texel, by its integer position (clamped to the frame)
fn texel_at(ip: vec2<f32>, sz: vec2<f32>) -> vec3<f32> {
    let q = clamp(ip, vec2(0.0), sz - vec2(1.0));
    return raw_rgb((q + vec2(0.5)) / sz, 0.0, true);
}

fn easu_luma(c: vec3<f32>) -> f32 {
    return c.b * 0.5 + (c.r * 0.5 + c.g);
}

fn easu_set(dir: ptr<function, vec2<f32>>, len: ptr<function, f32>, w: f32, a: f32, b: f32, c: f32, d: f32, e: f32) {
    let lx = 1.0 / max(max(abs(d - c), abs(c - b)), 1.0 / 32768.0);
    let dx = d - b;
    (*dir).x += dx * w;
    var lenx = clamp(abs(dx) * lx, 0.0, 1.0);
    lenx = lenx * lenx;
    *len += lenx * w;
    let ly = 1.0 / max(max(abs(e - c), abs(c - a)), 1.0 / 32768.0);
    let dy = e - a;
    (*dir).y += dy * w;
    var leny = clamp(abs(dy) * ly, 0.0, 1.0);
    leny = leny * leny;
    *len += leny * w;
}

fn easu_tap(acc: ptr<function, vec3<f32>>, accw: ptr<function, f32>, off: vec2<f32>, dir: vec2<f32>, len2: vec2<f32>, lob: f32, clp: f32, c: vec3<f32>) {
    var v = vec2(off.x * dir.x + off.y * dir.y, off.x * (-dir.y) + off.y * dir.x);
    v = v * len2;
    var d2 = min(v.x * v.x + v.y * v.y, clp);
    var wb = 2.0 / 5.0 * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb = wb * wb;
    wa = wa * wa;
    wb = 25.0 / 16.0 * wb - (25.0 / 16.0 - 1.0);
    let w = wb * wa;
    *acc += c * w;
    *accw += w;
}

fn easu(uv: vec2<f32>) -> vec3<f32> {
    let sz = src_size();
    let pos = uv * sz - vec2(0.5);
    let fp = floor(pos);
    let pp = pos - fp;
    //      b c
    //    e f g h
    //    i j k l
    //      n o
    let cb = texel_at(fp + vec2(0.0, -1.0), sz);
    let cc = texel_at(fp + vec2(1.0, -1.0), sz);
    let ce = texel_at(fp + vec2(-1.0, 0.0), sz);
    let cf = texel_at(fp, sz);
    let cg = texel_at(fp + vec2(1.0, 0.0), sz);
    let ch = texel_at(fp + vec2(2.0, 0.0), sz);
    let ci = texel_at(fp + vec2(-1.0, 1.0), sz);
    let cj = texel_at(fp + vec2(0.0, 1.0), sz);
    let ck = texel_at(fp + vec2(1.0, 1.0), sz);
    let cl = texel_at(fp + vec2(2.0, 1.0), sz);
    let cn = texel_at(fp + vec2(0.0, 2.0), sz);
    let co = texel_at(fp + vec2(1.0, 2.0), sz);
    let lb = easu_luma(cb); let lc = easu_luma(cc); let le = easu_luma(ce); let lf = easu_luma(cf);
    let lg = easu_luma(cg); let lh = easu_luma(ch); let li = easu_luma(ci); let lj = easu_luma(cj);
    let lk = easu_luma(ck); let ll = easu_luma(cl); let ln = easu_luma(cn); let lo = easu_luma(co);
    var dir = vec2(0.0);
    var len = 0.0;
    easu_set(&dir, &len, (1.0 - pp.x) * (1.0 - pp.y), lb, le, lf, lg, lj);
    easu_set(&dir, &len, pp.x * (1.0 - pp.y), lc, lf, lg, lh, lk);
    easu_set(&dir, &len, (1.0 - pp.x) * pp.y, lf, li, lj, lk, ln);
    easu_set(&dir, &len, pp.x * pp.y, lg, lj, lk, ll, lo);
    let dir2 = dir * dir;
    var dirr = dir2.x + dir2.y;
    let zro = dirr < 1.0 / 32768.0;
    dirr = inverseSqrt(max(dirr, 1.0 / 32768.0));
    dirr = select(dirr, 1.0, zro);
    dir.x = select(dir.x, 1.0, zro);
    dir = dir * dirr;
    len = len * 0.5;
    len = len * len;
    let stretch = (dir.x * dir.x + dir.y * dir.y) / max(abs(dir.x), abs(dir.y));
    let len2 = vec2(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    let lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
    let clp = 1.0 / lob;
    var acc = vec3(0.0);
    var accw = 0.0;
    easu_tap(&acc, &accw, vec2(0.0, -1.0) - pp, dir, len2, lob, clp, cb);
    easu_tap(&acc, &accw, vec2(1.0, -1.0) - pp, dir, len2, lob, clp, cc);
    easu_tap(&acc, &accw, vec2(-1.0, 0.0) - pp, dir, len2, lob, clp, ce);
    easu_tap(&acc, &accw, vec2(0.0, 0.0) - pp, dir, len2, lob, clp, cf);
    easu_tap(&acc, &accw, vec2(1.0, 0.0) - pp, dir, len2, lob, clp, cg);
    easu_tap(&acc, &accw, vec2(2.0, 0.0) - pp, dir, len2, lob, clp, ch);
    easu_tap(&acc, &accw, vec2(-1.0, 1.0) - pp, dir, len2, lob, clp, ci);
    easu_tap(&acc, &accw, vec2(0.0, 1.0) - pp, dir, len2, lob, clp, cj);
    easu_tap(&acc, &accw, vec2(1.0, 1.0) - pp, dir, len2, lob, clp, ck);
    easu_tap(&acc, &accw, vec2(2.0, 1.0) - pp, dir, len2, lob, clp, cl);
    easu_tap(&acc, &accw, vec2(0.0, 2.0) - pp, dir, len2, lob, clp, cn);
    easu_tap(&acc, &accw, vec2(1.0, 2.0) - pp, dir, len2, lob, clp, co);
    let mn = min(min(cf, cg), min(cj, ck));
    let mx = max(max(cf, cg), max(cj, ck));
    return clamp(acc / max(accw, 1e-5), mn, mx);
}

// EASU, then RCAS: the sharpen reads the upscaled picture one output pixel away on each side
fn fsr(uv: vec2<f32>) -> vec3<f32> {
    let step = (p.uv.zw - p.uv.xy) / max(p.rect.zw, vec2(1.0));
    let e = easu(uv);
    let b = easu(uv - vec2(0.0, step.y));
    let d = easu(uv - vec2(step.x, 0.0));
    let f = easu(uv + vec2(step.x, 0.0));
    let h = easu(uv + vec2(0.0, step.y));
    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));
    let hit_min = mn4 / (4.0 * max(mx4, vec3(1e-4)));
    let hit_max = (vec3(1.0) - mx4) / (4.0 * mn4 - vec3(4.0) - vec3(1e-4) * 0.0);
    let lobe_rgb = max(-hit_min, hit_max);
    // sharpness in stops held back (look.x): 0 is the sharpest FSR allows, 0.2 the usual default
    let con = exp2(-p.look.x);
    let lobe = max(-0.1875, min(max(lobe_rgb.r, max(lobe_rgb.g, lobe_rgb.b)), 0.0)) * con;
    let rcp_l = 1.0 / (4.0 * lobe + 1.0);
    return clamp(((b + d + f + h) * lobe + e) * rcp_l, vec3(0.0), vec3(1.0));
}

// where the mask says nobody is, the picture is blurred (a spiral of taps of `look.y` texels' radius over the source)
fn keep_person(uv: vec2<f32>, fg: vec3<f32>) -> vec3<f32> {
    let m = textureSampleLevel(mask, smooth_s, uv, 0.0).r;
    let a = smoothstep(0.30, 0.70, m);
    if a >= 0.999 {
        return fg;
    }
    let t = texel_step();
    let r = p.look.y;
    var acc = vec3(0.0);
    for (var i = 0; i < 24; i = i + 1) {
        let ang = f32(i) * 2.39996323;
        let rr = r * sqrt((f32(i) + 0.5) / 24.0);
        acc += raw_rgb(uv + vec2(cos(ang), sin(ang)) * rr * t, 0.0, false);
    }
    var bg = acc / 24.0;
    if (p.flags & 4u) != 0u {
        bg = grade(bg);
    }
    return mix(bg, fg, a);
}

@fragment
fn fs(v: V) -> @location(0) vec4<f32> {
    var c: vec4<f32>;
    if (p.flags & 8u) != 0u {
        // FSR 1 (flags bit 3): the upscaled picture, then the correction without its own unsharp (RCAS sharpened)
        var u = fsr(v.uv);
        if (p.flags & 4u) != 0u {
            u = grade(u);
        }
        c = vec4(u, 1.0);
    } else if p.mode == 1u {
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
    if (p.flags & 4u) != 0u && (p.flags & 8u) == 0u {
        c = vec4(graded_sharp(v.uv, c.rgb), c.a);
    }
    // a person in front of a blurred background (flags bit 4): the mask keeps what it covers sharp
    if (p.flags & 16u) != 0u {
        c = vec4(keep_person(v.uv, c.rgb), c.a);
    }
    // premultiplied, thinned out by the picture's own opacity
    return c * p.alpha;
}

// the correction: white balance and exposure by a gain per channel, the levels, the middle tones lifted, the colour
fn grade(c: vec3<f32>) -> vec3<f32> {
    var x = c * p.grade.xyz;
    x = clamp((x - vec3(p.grade2.x)) / max(p.grade2.y - p.grade2.x, 0.05), vec3(0.0), vec3(1.0));
    x = pow(x, vec3(max(p.grade.w, 0.05)));
    let l = dot(x, vec3(0.299, 0.587, 0.114));
    return clamp(mix(vec3(l), x, p.grade2.w), vec3(0.0), vec3(1.0));
}

// one texel in uv units (a YUYV texel holds two pixels across)
fn texel_step() -> vec2<f32> {
    let d = vec2<f32>(textureDimensions(tex));
    return select(1.0 / d, vec2(0.5 / d.x, 1.0 / d.y), p.mode == 2u);
}

// the correction and, over it, an unsharp mask (the picture less its blur, added back)
fn graded_sharp(uv: vec2<f32>, c: vec3<f32>) -> vec3<f32> {
    let t = texel_step();
    let b = (raw_rgb(uv + vec2(t.x, 0.0), 0.0, false) + raw_rgb(uv - vec2(t.x, 0.0), 0.0, false) + raw_rgb(uv + vec2(0.0, t.y), 0.0, false) + raw_rgb(uv - vec2(0.0, t.y), 0.0, false)) * 0.25;
    let g = grade(c);
    return clamp(g + (g - grade(b)) * p.grade2.z, vec3(0.0), vec3(1.0));
}
"; // (the glass half of the shader follows)

const SHADER_GLASS: &str = r#"
// ---- glass over the picture: the same lens, blur, tint and lit rim as `Canvas::glass_round_rect`, made here because the
// picture is not on the processor's canvas. Drawn as a quad over each panel, after the picture.

// the picture's colour at a place on the screen (px); the ground beyond its edges
fn raw_rgb(uv: vec2<f32>, lod: f32, crisp: bool) -> vec3<f32> {
    if p.mode == 1u {
        let y = textureSampleLevel(tex, smooth_s, uv, 0.0).r;
        let c = textureSampleLevel(tex2, smooth_s, uv, 0.0).rg;
        return yuv(y, c.x, c.y);
    } else if p.mode == 2u {
        let d = textureDimensions(tex);
        let ix = clamp(i32(floor(uv.x * f32(d.x * 2u))), 0, i32(d.x * 2u) - 1);
        let iy = clamp(i32(floor(uv.y * f32(d.y))), 0, i32(d.y) - 1);
        let t = textureLoad(tex, vec2<i32>(ix / 2, iy), 0);
        return yuv(select(t.r, t.b, (ix % 2) == 1), t.g, t.a);
    }
    if crisp {
        return textureSampleLevel(tex, crisp_s, uv, 0.0).rgb;
    }
    return textureSampleLevel(tex, smooth_s, uv, lod).rgb;
}

fn pic_rgb(uv: vec2<f32>, lod: f32, crisp: bool) -> vec3<f32> {
    let c = raw_rgb(uv, lod, crisp);
    if (p.flags & 4u) != 0u {
        return grade(c);
    }
    return c;
}

fn pic_at(pos: vec2<f32>, lod: f32, crisp: bool) -> vec3<f32> {
    let c = (pos - p.pic.xy) / p.pic.zw;
    if c.x < 0.0 || c.y < 0.0 || c.x > 1.0 || c.y > 1.0 {
        return p.ground.rgb;
    }
    return pic_rgb(mix(p.uv.xy, p.uv.zw, c), lod, crisp);
}

// a soft blur: a golden-angle spiral of taps, from the mip level whose texels are about as far apart as the taps
fn blur_px(pos: vec2<f32>, radius: f32) -> vec3<f32> {
    let k = max(p.gfx.y, 0.0001);
    let lod = clamp(log2(max(0.36 * radius / k, 1.0)), p.lod, max(p.gfx.x, p.lod));
    var acc = vec3<f32>(0.0);
    for (var i = 0; i < 16; i = i + 1) {
        let a = f32(i) * 2.39996323;
        let r = radius * sqrt((f32(i) + 0.5) / 16.0);
        acc += pic_at(pos + vec2(cos(a), sin(a)) * r, lod, false);
    }
    return acc / 16.0;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

// the shell's convex-lens height profile (glass.wgsl): 0 at the outer edge, 1 at the inner bezel limit
fn lens(t: f32) -> f32 {
    let u = clamp(t, 0.0, 1.0);
    return sqrt(1.0 - (1.0 - u) * (1.0 - u));
}

// The same glass as the shell's (`shade_glass` in glass.wgsl, its liquid look): the backdrop bent in a band at the edge
// by the lens, with a little colour dispersion, blurred, saturated, tinted; lit from above by a crisp rim.
@fragment
fn fs_glass(v: V) -> @location(0) vec4<f32> {
    let pos = v.pos.xy;
    let hf = p.bx.zw;
    let m = min(hf.x, hf.y);
    var r = min(p.glass.x, m);
    // smoothed (squircle) corners, as `canvas::corner_shape` makes them: look2.w is the smoothing
    var ex = 2.0;
    let ks = p.look2.w * clamp((0.92 * m - r) / (0.3 * m), 0.0, 1.0);
    if ks > 0.0 && r >= 0.75 {
        r = min(r * (1.0 + 0.45 * ks), m);
        ex = 2.0 + 2.6 * ks;
    }
    let d0 = pos - p.bx.xy;
    let q = abs(d0) - hf + vec2(r);
    var d = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
    if ex > 2.0 && q.x > 0.0 && q.y > 0.0 {
        d = pow(pow(q.x, ex) + pow(q.y, ex), 1.0 / ex) - r;
    }
    let fade = clamp(p.glass.y, 0.0, 1.0);
    let cov = clamp(0.5 - d, 0.0, 1.0) * fade;
    if cov <= 0.0 {
        return vec4(0.0);
    }
    let sc = p.glass.w;
    let depth = -d;
    // the outward normal of the rounded rectangle
    var n = vec2(0.0, sign(d0.y));
    if q.x > 0.0 && q.y > 0.0 {
        let g = pow(q, vec2(ex - 1.0));
        n = g / length(g) * sign(d0);
    } else if q.x > q.y {
        n = vec2(sign(d0.x), 0.0);
    }
    // liquid glass bends the scene only in a band at the rim: at most 45 % of the short side
    let short_side = min(hf.x, hf.y) * 2.0;
    let bezel = min(min(p.look.z * sc, short_side * 0.5), short_side * 0.45);
    let refr = min(p.look.y * sc, min(bezel * 1.15, short_side * 0.45));
    let t = depth / max(bezel, 1.0);
    // (the square of the distance to the band's inner limit: the convex-lens profile bends only the last pixel or two)
    let k = 1.0 - clamp(t, 0.0, 1.0);
    let k2 = select(0.0, k * k, t < 1.0);
    let disp = -n * k2 * refr;
    // the bend shows only where there is something to bend: the blur is mostly taken off in the band at the rim
    let blur_r = max(p.look.x * sc * 1.1 * (1.0 - 0.8 * k2), 1.0);
    var col = blur_px(pos + disp, blur_r);
    if k2 > 0.0001 && p.look.w >= 0.001 {
        col.r = blur_px(pos + disp * (1.0 + p.look.w), blur_r).r;
        col.b = blur_px(pos + disp * (1.0 - p.look.w), blur_r).b;
    }
    col = mix(vec3(luma(col)), col, select(p.look2.z, 1.15, p.look2.z <= 0.0));
    col = mix(col, p.tint.rgb, p.tint.a) + vec3(0.015);
    // the rim: a crisp bright line along the top edge, a fainter one along the bottom, the sides almost dark
    let dl = depth / sc;
    if dl < 40.0 {
        let nd = dot(n, normalize(vec2(-0.22, -1.0)));
        let edge = exp(-dl / 0.6);
        let inner = exp(-dl / 3.2);
        let top = pow(max(nd, 0.0), 2.2);
        let bottom = pow(max(-nd, 0.0), 3.0) * 0.42;
        let rim_k = mix(1.0, 0.45, smoothstep(0.45, 0.9, luma(col)));
        col += vec3(1.0) * rim_k * (edge * (0.10 + 0.85 * (top + bottom)) + inner * 0.10 * top) * p.look2.x;
    }
    // a 1px inner border in the theme's border colour
    let band = 1.0 - smoothstep(0.0, 1.5, dl);
    col = mix(col, p.border.rgb, p.border.a * band);
    return vec4(clamp(col, vec3(0.0), vec3(1.0)) * cov, cov);
}
"#;

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
    pic: [f32; 4],
    ground: [f32; 4],
    glass: [f32; 4],
    gfx: [f32; 4],
    bx: [f32; 4],
    look: [f32; 4],
    look2: [f32; 4],
    tint: [f32; 4],
    border: [f32; 4],
    grade: [f32; 4],
    grade2: [f32; 4],
}

/// What was drawn last: when nothing differs, the frame is not drawn again.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    live: bool,
    /// a program's scene (`pic` is then its version)
    scene: bool,
    mirror: bool,
    pic: u64,
    size: [u32; 2],
    rect: [f32; 4],
    src: [f32; 4],
    alpha: f32,
    ground: [f32; 3],
    grade: Option<crate::live::Grade>,
    /// FSR on, its sharpness, the mask shown and the blur radius (as bits): what else changes how a frame looks
    fx: (bool, u32, u64, u32),
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
    /// the glass panels over the picture (see `SHADER_GLASS`)
    glass_pipeline: wgpu::RenderPipeline,
    /// the panels drawn last, with `drawn`
    drawn_glass: Vec<GlassRect>,
    layout: wgpu::BindGroupLayout,
    smooth: wgpu::Sampler,
    crisp: wgpu::Sampler,
    textures: Vec<Uploaded>,
    /// the live frame's textures, updated in place
    live: Option<LiveTex>,
    /// the person mask of the live frame (R8), its id and size
    mask_tex: Option<(wgpu::Texture, wgpu::TextureView, u64, [u32; 2])>,
    /// bound as the second plane when there is none
    dummy: wgpu::TextureView,
    max_side: u32,
    frame: u64,
    drawn: Option<Drawn>,
    /// the texture a program's scene is drawn into, made again when the size changes
    scene_target: Option<([u32; 2], wgpu::TextureView)>,
    /// tells a scene which device this is
    device_id: u64,
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

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("picture"), source: wgpu::ShaderSource::Wgsl(format!("{SHADER}{SHADER_GLASS}").into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
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
        let glass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glass over the picture"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_glass"),
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
        Ok(GpuLayer { surface: surf, device, queue, config, configured: [0, 0], pipeline, glass_pipeline, drawn_glass: Vec::new(), layout, smooth, crisp, textures: Vec::new(), live: None, mask_tex: None, dummy, max_side, frame: 0, drawn: None, scene_target: None, device_id: crate::scene::next_device_id() })
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
    pub fn render(&mut self, size: [u32; 2], scale: f32, l: &PictureLayer, glass: &[GlassRect]) -> Result<(), String> {
        let rect = [l.dest[0] * scale, l.dest[1] * scale, l.dest[2] * scale, l.dest[3] * scale];
        let now = Drawn { live: false, scene: false, mirror: false, pic: l.pic.id, size, rect, src: l.src, alpha: l.alpha, ground: l.ground, grade: None, fx: (false, 0, 0, 0) };
        if self.drawn == Some(now) && self.drawn_glass == glass {
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
            pic: rect,
            ground: [l.ground[0], l.ground[1], l.ground[2], 1.0],
            glass: [0.0; 4],
            gfx: [(tex.mips - 1) as f32, k, 0.0, 0.0],
            bx: [0.0; 4],
            look: [0.0; 4],
            look2: [0.0; 4],
            tint: [0.0; 4],
            border: [0.0; 4],
            grade: [1.0, 1.0, 1.0, 1.0],
            grade2: [0.0, 1.0, 0.0, 1.0],
        };
        self.paint(params, &tex.view, &self.dummy, &self.dummy, l.ground, glass, scale)?;
        self.drawn = Some(now);
        self.drawn_glass = glass.to_vec();
        Ok(())
    }

    /// Draw a program's scene into a texture of `l.dest`'s size, then show it like a picture. Nothing is drawn when the
    /// version, the size and the glass are what they were.
    pub fn render_scene(&mut self, size: [u32; 2], scale: f32, l: &crate::scene::SceneLayer, glass: &[GlassRect]) -> Result<(), String> {
        let rect = [l.dest[0] * scale, l.dest[1] * scale, l.dest[2] * scale, l.dest[3] * scale];
        let now = Drawn { live: false, scene: true, mirror: false, pic: l.version, size, rect, src: [0.0; 4], alpha: 1.0, ground: l.ground, grade: None, fx: (false, 0, 0, 0) };
        if self.drawn == Some(now) && self.drawn_glass == glass {
            return Ok(());
        }
        let tsize = [(rect[2].round() as u32).clamp(1, self.max_side), (rect[3].round() as u32).clamp(1, self.max_side)];
        if self.scene_target.as_ref().map(|t| t.0) != Some(tsize) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("scene"),
                size: wgpu::Extent3d { width: tsize[0], height: tsize[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: crate::scene::FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.scene_target = Some((tsize, texture.create_view(&wgpu::TextureViewDescriptor::default())));
        }
        let Some((_, target)) = &self.scene_target else { return Err("no scene texture".into()) };
        {
            let gpu = crate::scene::Gpu { device: &self.device, queue: &self.queue, id: self.device_id };
            let mut scene = l.scene.lock().map_err(|_| "the scene's lock is poisoned".to_string())?;
            scene.render(&gpu, target, tsize);
        }
        self.fit_surface(size);
        let params = Params {
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            view: [size[0] as f32, size[1] as f32],
            alpha: 1.0,
            lod: 0.0,
            nearest: 0,
            mode: 0,
            flags: 0,
            _pad: 0,
            pic: rect,
            ground: [l.ground[0], l.ground[1], l.ground[2], 1.0],
            glass: [0.0; 4],
            gfx: [0.0, 1.0, 0.0, 0.0],
            bx: [0.0; 4],
            look: [0.0; 4],
            look2: [0.0; 4],
            tint: [0.0; 4],
            border: [0.0; 4],
            grade: [1.0, 1.0, 1.0, 1.0],
            grade2: [0.0, 1.0, 0.0, 1.0],
        };
        let target = self.scene_target.as_ref().map(|t| &t.1).ok_or("no scene texture")?;
        self.paint(params, target, &self.dummy, &self.dummy, l.ground, glass, scale)?;
        self.drawn = Some(now);
        self.drawn_glass = glass.to_vec();
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

    /// Put a person mask into its texture (made when the size is new, written when the mask is).
    fn mask_upload(&mut self, m: &crate::live::Mask) {
        if m.w == 0 || m.h == 0 || m.data.len() < (m.w * m.h) as usize {
            return;
        }
        if self.mask_tex.as_ref().is_none_or(|t| t.3 != [m.w, m.h]) {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("person mask"),
                size: wgpu::Extent3d { width: m.w, height: m.h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.mask_tex = Some((tex, view, 0, [m.w, m.h]));
        }
        let Some(t) = self.mask_tex.as_mut().filter(|t| t.2 != m.id) else { return };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &t.0, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &m.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(m.w), rows_per_image: Some(m.h) },
            wgpu::Extent3d { width: m.w, height: m.h, depth_or_array_layers: 1 },
        );
        t.2 = m.id;
        self.drawn = None;
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
    pub fn render_live(&mut self, size: [u32; 2], scale: f32, l: &LiveLayer, glass: &[GlassRect]) -> Result<(), String> {
        let f = &l.frame;
        let rect = [l.dest[0] * scale, l.dest[1] * scale, l.dest[2] * scale, l.dest[3] * scale];
        let fx = (l.fsr, l.fsr_stops.to_bits(), l.blur.as_ref().map_or(0, |b| b.0.id), l.blur.as_ref().map_or(0, |b| b.1.to_bits()));
        let now = Drawn { live: true, scene: false, mirror: l.mirror, pic: f.id, size, rect, src: l.src, alpha: l.alpha, ground: l.ground, grade: l.grade, fx };
        if self.drawn == Some(now) && self.drawn_glass == glass {
            return Ok(());
        }
        self.live_textures(f)?;
        self.live_upload(f);
        if let Some((m, _)) = &l.blur {
            self.mask_upload(m);
        }
        self.fit_surface(size);
        let Some(t) = &self.live else { return Err("no live textures".into()) };
        let mask_view = self.mask_tex.as_ref().filter(|_| l.blur.is_some()).map(|m| &m.1).unwrap_or(&self.dummy);
        let (fw, fh) = (f.w as f32, f.h as f32);
        let (mut u0, mut u1) = (l.src[0] / fw, (l.src[0] + l.src[2]) / fw);
        if l.mirror {
            std::mem::swap(&mut u0, &mut u1);
        }
        let k = (l.dest[2] / l.src[2].max(1e-6) * scale).min(l.dest[3] / l.src[3].max(1e-6) * scale);
        let mut params = Params {
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
            flags: f.color.limited as u32 | (f.color.bt709 as u32) << 1 | if l.grade.is_some() { 4 } else { 0 } | if l.fsr && k > 1.05 && k <= 3.0 { 8 } else { 0 } | if l.blur.is_some() { 16 } else { 0 },
            _pad: 0,
            pic: rect,
            ground: [l.ground[0], l.ground[1], l.ground[2], 1.0],
            glass: [0.0; 4],
            gfx: [0.0, k, 0.0, 0.0],
            bx: [0.0; 4],
            look: [l.fsr_stops, l.blur.as_ref().map_or(0.0, |b| b.1), 0.0, 0.0],
            look2: [0.0; 4],
            tint: [0.0; 4],
            border: [0.0; 4],
            grade: [1.0, 1.0, 1.0, 1.0],
            grade2: [0.0, 1.0, 0.0, 1.0],
        };
        if let Some(g) = l.grade {
            params.grade = [g.gain[0], g.gain[1], g.gain[2], g.gamma];
            params.grade2 = [g.black, g.white, g.sharpen, g.saturation];
        }
        self.paint(params, &t.main_view, t.chroma_view.as_ref().unwrap_or(&self.dummy), mask_view, l.ground, glass, scale)?;
        self.drawn = Some(now);
        self.drawn_glass = glass.to_vec();
        Ok(())
    }

    /// One textured quad over the ground colour and, over it, a panel of glass for each of `glass` (logical px, `scale`
    /// device px each), presented.
    fn paint(&self, params: Params, main: &wgpu::TextureView, chroma: &wgpu::TextureView, mask: &wgpu::TextureView, ground: [f32; 3], glass: &[GlassRect], scale: f32) -> Result<(), String> {
        // one uniform block per draw, in one buffer, picked with a dynamic offset
        let size = std::mem::size_of::<Params>();
        let align = self.device.limits().min_uniform_buffer_offset_alignment as usize;
        let stride = size.div_ceil(align) * align;
        let mut bytes = vec![0u8; stride * (1 + glass.len())];
        bytes[..size].copy_from_slice(bytemuck::bytes_of(&params));
        for (i, g) in glass.iter().enumerate() {
            let px = [g.r[0] * scale, g.r[1] * scale, g.r[2] * scale, g.r[3] * scale];
            let mut q = params;
            // the quad is a pixel wider than the panel, for its anti-aliased edge
            q.rect = [px[0] - 1.0, px[1] - 1.0, px[2] + 2.0, px[3] + 2.0];
            q.glass = [g.radius * scale, g.strength, if g.dark { 1.0 } else { 0.0 }, scale];
            q.bx = [px[0] + px[2] / 2.0, px[1] + px[3] / 2.0, px[2] / 2.0, px[3] / 2.0];
            q.look = [g.look[0], g.look[1], g.look[2], g.look[3]];
            q.look2 = [g.look[4], g.look[5], g.look[6], crate::canvas::corner_smoothing()];
            q.tint = g.tint;
            q.border = g.border;
            bytes[(i + 1) * stride..(i + 1) * stride + size].copy_from_slice(bytemuck::bytes_of(&q));
        }
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("picture"), contents: &bytes, usage: wgpu::BufferUsages::UNIFORM });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("picture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &uniform, offset: 0, size: wgpu::BufferSize::new(size as u64) }) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(main) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.smooth) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.crisp) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(chroma) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(mask) },
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
            pass.set_bind_group(0, &bind, &[0]);
            pass.draw(0..6, 0..1);
            if !glass.is_empty() {
                pass.set_pipeline(&self.glass_pipeline);
                for i in 0..glass.len() {
                    pass.set_bind_group(0, &bind, &[((i + 1) * stride) as u32]);
                    pass.draw(0..6, 0..1);
                }
            }
        }
        self.queue.submit([enc.finish()]);
        frame.present();
        Ok(())
    }

    /// The layer is hidden for now: the next `render` draws even if nothing differs.
    pub fn forget(&mut self) {
        self.drawn = None;
        self.drawn_glass.clear();
    }
}

#[cfg(test)]
mod tests {
    /// The picture layer's shaders are valid WGSL for the card.
    #[test]
    fn the_layer_shader_compiles() {
        let _one = crate::MEMORY_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(off) = crate::scene::Offscreen::new(true) else { return };
        off.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _ = off.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("picture"), source: wgpu::ShaderSource::Wgsl(format!("{}{}", super::SHADER, super::SHADER_GLASS).into()) });
        let err = pollster::block_on(off.device.pop_error_scope());
        assert!(err.is_none(), "{err:?}");
    }
}

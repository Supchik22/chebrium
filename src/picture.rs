//! Pictures: decoded images (and video frames) that can be drawn scaled, panned and zoomed.
//!
//! A [`Pic`] holds premultiplied BGRA pixels, the layout of the canvas, plus a chain of half-size copies (mips) so a
//! picture drawn much smaller than it is stays clean instead of aliasing. [`Canvas::blit_pic`] samples it with
//! bilinear filtering in fixed point (nearest once it is magnified a lot, so pixels stay crisp).

use crate::canvas::Canvas;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct Level {
    w: usize,
    h: usize,
    data: Vec<u8>,
}

/// An image ready to be drawn. Cheap to share (`Arc<Pic>`); two pictures are the same only if they are the same value.
pub struct Pic {
    pub id: u64,
    pub w: u32,
    pub h: u32,
    levels: Vec<Level>,
    /// every pixel is fully opaque (so drawing can copy instead of blend)
    opaque: bool,
    /// the picture is the whole window: drawn by replacing what is under it, alpha and all (a terminal that is
    /// translucent itself), so no blending is needed
    replace: bool,
}

impl PartialEq for Pic {
    fn eq(&self, other: &Pic) -> bool {
        self.id == other.id
    }
}

impl std::fmt::Debug for Pic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pic#{} {}x{}", self.id, self.w, self.h)
    }
}

fn premultiply_bgra(rgba: &[u8]) -> (Vec<u8>, bool) {
    let mut out = Vec::with_capacity(rgba.len());
    let mut opaque = true;
    for p in rgba.chunks_exact(4) {
        let a = p[3] as u32;
        opaque &= a == 255;
        out.extend_from_slice(&[((p[2] as u32 * a + 127) / 255) as u8, ((p[1] as u32 * a + 127) / 255) as u8, ((p[0] as u32 * a + 127) / 255) as u8, a as u8]);
    }
    (out, opaque)
}

/// Half-size copy by averaging 2 x 2 blocks.
fn halve(l: &Level) -> Level {
    let (w, h) = ((l.w / 2).max(1), (l.h / 2).max(1));
    let mut data = vec![0u8; w * h * 4];
    for y in 0..h {
        let (y0, y1) = ((y * 2).min(l.h - 1), (y * 2 + 1).min(l.h - 1));
        for x in 0..w {
            let (x0, x1) = ((x * 2).min(l.w - 1), (x * 2 + 1).min(l.w - 1));
            for c in 0..4 {
                let sum = l.data[(y0 * l.w + x0) * 4 + c] as u32 + l.data[(y0 * l.w + x1) * 4 + c] as u32 + l.data[(y1 * l.w + x0) * 4 + c] as u32 + l.data[(y1 * l.w + x1) * 4 + c] as u32;
                data[(y * w + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    Level { w, h, data }
}

impl Pic {
    fn build(w: u32, h: u32, data: Vec<u8>, opaque: bool, mips: bool) -> Pic {
        let mut levels = vec![Level { w: w as usize, h: h as usize, data }];
        while mips && levels.len() < 8 && levels.last().is_some_and(|l| l.w > 16 && l.h > 16) {
            let next = halve(levels.last().unwrap());
            levels.push(next);
        }
        Pic { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), w, h, levels, opaque, replace: false }
    }

    /// From straight (non-premultiplied) RGBA bytes, as decoders produce them.
    pub fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> Pic {
        let (data, opaque) = premultiply_bgra(rgba);
        Pic::build(w, h, data, opaque, true)
    }

    /// From BGRA bytes that are already in the canvas layout (`opaque`: every alpha is 255). `mips` builds the smaller
    /// copies: worth it for a still picture, not for a video frame that is shown once.
    pub fn from_bgra(w: u32, h: u32, data: Vec<u8>, opaque: bool, mips: bool) -> Pic {
        Pic::build(w, h, data, opaque, mips)
    }

    /// A picture that is the whole content of the window and may be translucent: it replaces what is under it instead of
    /// blending (premultiplied BGRA, the canvas layout). For programs that paint their own window.
    pub fn from_bgra_window(w: u32, h: u32, data: Vec<u8>) -> Pic {
        let mut p = Pic::build(w, h, data, false, false);
        p.replace = true;
        p
    }

    /// The picture turned by `quarter_turns` clockwise quarter turns.
    pub fn rotated(&self, quarter_turns: u32) -> Pic {
        let src = &self.levels[0];
        let t = quarter_turns % 4;
        if t == 0 {
            return Pic::build(self.w, self.h, src.data.clone(), self.opaque, true);
        }
        let (nw, nh) = if t == 2 { (src.w, src.h) } else { (src.h, src.w) };
        let mut data = vec![0u8; nw * nh * 4];
        for y in 0..src.h {
            for x in 0..src.w {
                let (dx, dy) = match t {
                    1 => (src.h - 1 - y, x),
                    2 => (src.w - 1 - x, src.h - 1 - y),
                    _ => (y, src.w - 1 - x),
                };
                data[(dy * nw + dx) * 4..(dy * nw + dx) * 4 + 4].copy_from_slice(&src.data[(y * src.w + x) * 4..(y * src.w + x) * 4 + 4]);
            }
        }
        Pic::build(nw as u32, nh as u32, data, self.opaque, true)
    }

    /// The pixels of the full-size picture: premultiplied BGRA, `w * h * 4` bytes, with the width and height.
    pub fn pixels(&self) -> (&[u8], usize, usize) {
        let l = &self.levels[0];
        (&l.data, l.w, l.h)
    }

    /// Approximate memory held, for deciding what to keep cached.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.data.len()).sum()
    }
}

impl Canvas<'_> {
    /// Draw the part `src` (x, y, w, h in picture pixels) of `pic` into the box `dest` (canvas pixels), scaled to fit it.
    pub fn blit_pic(&mut self, dest: [f32; 4], pic: &Pic, src: [f32; 4], alpha: f32) {
        if dest[2] <= 0.0 || dest[3] <= 0.0 || src[2] <= 0.0 || src[3] <= 0.0 || alpha <= 0.0 {
            return;
        }
        let clip = self.clip;
        let (cx0, cx1) = ((dest[0].floor() as i32).max(clip[0]), ((dest[0] + dest[2]).ceil() as i32).min(clip[2]));
        let (cy0, cy1) = ((dest[1].floor() as i32).max(clip[1]), ((dest[1] + dest[3]).ceil() as i32).min(clip[3]));
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        // pixels per source pixel; below 1 a smaller mip is used
        let (sx, sy) = (dest[2] / src[2], dest[3] / src[3]);
        let scale = sx.min(sy);
        let level = if scale < 1.0 { ((1.0 / scale).log2().floor() as usize).min(pic.levels.len() - 1) } else { 0 };
        let l = &pic.levels[level];
        let f = 1.0 / (1u32 << level) as f32;
        let nearest = scale > 3.0;
        let a256 = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
        let opaque_copy = (pic.opaque || pic.replace) && a256 >= 256;
        // one source pixel per canvas pixel, no offset (a video frame at its own size): copy the rows
        let aligned = |v: f32| (v - v.round()).abs() < 1e-3;
        if opaque_copy && level == 0 && (sx - 1.0).abs() < 1e-4 && (sy - 1.0).abs() < 1e-4 && [dest[0], dest[1], src[0], src[1]].iter().all(|&v| aligned(v)) {
            let (dx0, dy0) = (dest[0].round() as i32, dest[1].round() as i32);
            let (sx0, sy0) = (src[0].round() as i32, src[1].round() as i32);
            for py in cy0..cy1 {
                let sy = py - dy0 + sy0;
                if sy < 0 || sy as usize >= l.h {
                    continue;
                }
                // the part of the row that has a source pixel
                let x0 = cx0.max(dx0 - sx0);
                let x1 = cx1.min(dx0 - sx0 + l.w as i32);
                if x0 >= x1 {
                    continue;
                }
                let srow = &l.data[(sy as usize * l.w + (x0 - dx0 + sx0) as usize) * 4..][..(x1 - x0) as usize * 4];
                self.data[(py as usize * self.w + x0 as usize) * 4..][..(x1 - x0) as usize * 4].copy_from_slice(srow);
            }
            return;
        }
        // the horizontal sampling positions do not change from row to row: work them out once
        let cols: Vec<(usize, usize, u32)> = (cx0..cx1)
            .map(|px| {
                let u = (src[0] + (px as f32 + 0.5 - dest[0]) / sx) * f - 0.5;
                if nearest {
                    let i = (u + 0.5).floor().clamp(0.0, (l.w - 1) as f32) as usize;
                    (i * 4, i * 4, 0)
                } else {
                    let u0 = u.floor();
                    let (i0, i1) = ((u0.max(0.0) as usize).min(l.w - 1), ((u0 + 1.0).max(0.0) as usize).min(l.w - 1));
                    (i0 * 4, i1 * 4, ((u - u0) * 256.0) as u32)
                }
            })
            .collect();
        for py in cy0..cy1 {
            let v = (src[1] + (py as f32 + 0.5 - dest[1]) / sy) * f - 0.5;
            let (r0, r1, wy) = if nearest {
                let i = (v + 0.5).floor().clamp(0.0, (l.h - 1) as f32) as usize;
                (i, i, 0)
            } else {
                let v0 = v.floor();
                ((v0.max(0.0) as usize).min(l.h - 1), ((v0 + 1.0).max(0.0) as usize).min(l.h - 1), ((v - v0) * 256.0) as u32)
            };
            let (row0, row1) = (&l.data[r0 * l.w * 4..(r0 + 1) * l.w * 4], &l.data[r1 * l.w * 4..(r1 + 1) * l.w * 4]);
            let out = &mut self.data[(py as usize * self.w + cx0 as usize) * 4..(py as usize * self.w + cx1 as usize) * 4];
            for (d, &(i0, i1, wx)) in out.chunks_exact_mut(4).zip(&cols) {
                let mut px = [0u32; 4];
                for c in 0..4 {
                    let top = (row0[i0 + c] as u32 * (256 - wx) + row0[i1 + c] as u32 * wx) >> 8;
                    let bot = (row1[i0 + c] as u32 * (256 - wx) + row1[i1 + c] as u32 * wx) >> 8;
                    px[c] = (top * (256 - wy) + bot * wy) >> 8;
                }
                if opaque_copy {
                    d.copy_from_slice(&[px[0] as u8, px[1] as u8, px[2] as u8, if pic.replace { px[3] as u8 } else { 255 }]);
                } else {
                    // premultiplied source-over, scaled by the picture's own alpha
                    let sa = (px[3] * a256) >> 8;
                    let inv = 256 - sa;
                    for c in 0..3 {
                        d[c] = (((px[c] * a256) >> 8) + ((d[c] as u32 * inv) >> 8)).min(255) as u8;
                    }
                    d[3] = (sa + ((d[3] as u32 * inv) >> 8)).min(255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pic {
        Pic::from_rgba(w, h, &rgba.repeat((w * h) as usize))
    }

    fn pixel(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let o = (y * w + x) * 4;
        [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]
    }

    #[test]
    fn a_picture_is_stored_as_the_canvas_wants_it() {
        let p = solid(4, 4, [10, 20, 30, 255]);
        let mut buf = vec![0u8; 8 * 8 * 4];
        Canvas::new(&mut buf, 8, 8).blit_pic([2.0, 2.0, 4.0, 4.0], &p, [0.0, 0.0, 4.0, 4.0], 1.0);
        assert_eq!(pixel(&buf, 8, 3, 3), [30, 20, 10, 255], "BGRA order");
        assert_eq!(pixel(&buf, 8, 1, 1), [0, 0, 0, 0], "nothing outside the box");
    }

    #[test]
    fn scaling_down_and_up_keeps_the_colour() {
        let p = solid(64, 64, [200, 100, 50, 255]);
        let mut buf = vec![0u8; 100 * 100 * 4];
        let mut c = Canvas::new(&mut buf, 100, 100);
        c.blit_pic([0.0, 0.0, 10.0, 10.0], &p, [0.0, 0.0, 64.0, 64.0], 1.0);
        c.blit_pic([20.0, 20.0, 80.0, 80.0], &p, [0.0, 0.0, 64.0, 64.0], 1.0);
        for (x, y) in [(5, 5), (50, 50), (99, 99)] {
            let q = pixel(&buf, 100, x, y);
            assert!((q[2] as i32 - 200).abs() <= 1 && (q[1] as i32 - 100).abs() <= 1 && (q[0] as i32 - 50).abs() <= 1, "{q:?}");
        }
    }

    #[test]
    fn a_crop_shows_only_that_part() {
        // left half red, right half blue
        let mut rgba = Vec::new();
        for _ in 0..8 {
            for x in 0..8 {
                rgba.extend_from_slice(if x < 4 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
            }
        }
        let p = Pic::from_rgba(8, 8, &rgba);
        let mut buf = vec![0u8; 16 * 16 * 4];
        Canvas::new(&mut buf, 16, 16).blit_pic([0.0, 0.0, 16.0, 16.0], &p, [4.0, 0.0, 4.0, 8.0], 1.0);
        assert_eq!(pixel(&buf, 16, 8, 8), [255, 0, 0, 255], "only the blue half was asked for");
    }

    #[test]
    fn the_clip_is_respected() {
        let p = solid(4, 4, [255, 255, 255, 255]);
        let mut buf = vec![0u8; 8 * 8 * 4];
        let mut c = Canvas::new(&mut buf, 8, 8);
        c.set_clip([0, 0, 4, 8]);
        c.blit_pic([0.0, 0.0, 8.0, 8.0], &p, [0.0, 0.0, 4.0, 4.0], 1.0);
        assert_eq!(pixel(&buf, 8, 6, 3), [0, 0, 0, 0]);
        assert_eq!(pixel(&buf, 8, 2, 3), [255, 255, 255, 255]);
    }

    #[test]
    fn rotating_turns_the_picture() {
        // a 2 x 1 picture, red then blue: a clockwise quarter turn makes it 1 x 2 with red on top
        let p = Pic::from_rgba(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]);
        let r = p.rotated(1);
        assert_eq!((r.w, r.h), (1, 2));
        assert_eq!(&r.levels[0].data[0..4], &[0, 0, 255, 255], "red (BGRA) is first");
        let back = r.rotated(3);
        assert_eq!(&back.levels[0].data[..], &p.levels[0].data[..]);
    }

    #[test]
    fn a_picture_at_its_own_size_is_copied_exactly() {
        let mut rgba = Vec::new();
        for i in 0..(6 * 4) {
            rgba.extend_from_slice(&[i as u8 * 9, 200 - i as u8, i as u8, 255]);
        }
        let p = Pic::from_rgba(6, 4, &rgba);
        let mut buf = vec![0u8; 10 * 8 * 4];
        Canvas::new(&mut buf, 10, 8).blit_pic([2.0, 3.0, 6.0, 4.0], &p, [0.0, 0.0, 6.0, 4.0], 1.0);
        for y in 0..4 {
            for x in 0..6 {
                let i = (y * 6 + x) as u8;
                assert_eq!(pixel(&buf, 10, 2 + x, 3 + y), [i, 200 - i, i * 9, 255], "pixel {x},{y}");
            }
        }
        assert_eq!(pixel(&buf, 10, 1, 3), [0, 0, 0, 0]);
    }
}

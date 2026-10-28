//! Tiny software canvas over a premultiplied ARGB8888 (little-endian: B,G,R,A) buffer.

pub type Rgb = [u8; 3];

/// One rasterised glyph: R8 coverage with its offset from the pen origin (fontdue's metrics).
pub struct Glyph {
    pub xmin: i32,
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub bitmap: Vec<u8>,
}

/// Straight 0..1 float colour to bytes.
pub fn rgb8(c: [f32; 3]) -> Rgb {
    [(c[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (c[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (c[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]
}

pub struct Canvas<'a> {
    pub data: &'a mut [u8],
    pub w: usize,
    pub h: usize,
    /// painting is limited to this box (x0, y0, x1, y1), the whole canvas unless a damage rectangle is being redrawn
    pub(crate) clip: [i32; 4],
}

#[inline]
fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

impl<'a> Canvas<'a> {
    pub fn new(data: &'a mut [u8], w: usize, h: usize) -> Self {
        Canvas { data, w, h, clip: [0, 0, w as i32, h as i32] }
    }

    /// Limit painting to `r` (x0, y0, x1, y1), clamped to the canvas.
    pub fn set_clip(&mut self, r: [i32; 4]) {
        self.clip = [r[0].max(0), r[1].max(0), r[2].min(self.w as i32), r[3].min(self.h as i32)];
    }

    pub fn reset_clip(&mut self) {
        self.clip = [0, 0, self.w as i32, self.h as i32];
    }

    /// Paint over the box `r` (x0, y0, x1, y1) with a solid colour, replacing what was there.
    pub fn clear_rect(&mut self, r: [i32; 4], c: Rgb, a: f32) {
        let px = [(c[2] as f32 * a + 0.5) as u8, (c[1] as f32 * a + 0.5) as u8, (c[0] as f32 * a + 0.5) as u8, (a * 255.0 + 0.5) as u8];
        let (x0, x1) = (r[0].max(0) as usize, (r[2].max(0) as usize).min(self.w));
        for y in (r[1].max(0) as usize)..(r[3].max(0) as usize).min(self.h) {
            for p in self.data[(y * self.w + x0) * 4..(y * self.w + x1.max(x0)) * 4].chunks_exact_mut(4) {
                p.copy_from_slice(&px);
            }
        }
    }

    /// Source-over blend of a straight (non-premultiplied) colour with alpha `a` (0..1).
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < self.clip[0] || y < self.clip[1] || x >= self.clip[2] || y >= self.clip[3] || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let i = (y as usize * self.w + x as usize) * 4;
        let inv = 1.0 - a;
        let d = &mut self.data[i..i + 4];
        d[0] = (c[2] as f32 * a + d[0] as f32 * inv + 0.5) as u8;
        d[1] = (c[1] as f32 * a + d[1] as f32 * inv + 0.5) as u8;
        d[2] = (c[0] as f32 * a + d[2] as f32 * inv + 0.5) as u8;
        d[3] = (255.0 * a + d[3] as f32 * inv + 0.5) as u8;
    }

    pub fn clear(&mut self, c: Rgb, a: f32) {
        let px = [(c[2] as f32 * a + 0.5) as u8, (c[1] as f32 * a + 0.5) as u8, (c[0] as f32 * a + 0.5) as u8, (a * 255.0 + 0.5) as u8];
        for chunk in self.data.chunks_exact_mut(4) {
            chunk.copy_from_slice(&px);
        }
    }

    pub fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb, a: f32) {
        for y in y0.max(0)..y1.min(self.h as i32) {
            for x in x0.max(0)..x1.min(self.w as i32) {
                self.blend(x, y, c, a);
            }
        }
    }

    /// Anti-aliased rounded rectangle. Along a row, everything between the two corner zones has the same coverage
    /// (it only depends on how far the row is from the top or bottom edge), so that middle span is filled in one go
    /// and only the few pixels at the ends need the distance maths: a big flat panel costs little.
    pub fn fill_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgb, a: f32) {
        let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let r = r.min(hw).min(hh);
        // past this margin from the sides a pixel's coverage does not depend on x
        let m = r.max(1.0);
        let clip = self.clip;
        let mx0 = ((cx - hw + m).ceil() as i32).max(clip[0]);
        let mx1 = ((cx + hw - m).floor() as i32).min(clip[2]);
        let start = (x.floor() as i32).max(clip[0]);
        let end = ((x + w).ceil() as i32).min(clip[2]);
        for py in (y.floor() as i32).max(clip[1])..((y + h).ceil() as i32).min(clip[3]) {
            let mut px = start;
            if mx0 < mx1 {
                // the part before the span, the span, then the part after it
                while px < mx0.min(end) {
                    self.edge_pixel(px, py, cx, cy, hw, hh, r, c, a);
                    px += 1;
                }
                let d = sd_round_rect(mx0 as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r);
                let row_a = a * (0.5 - d).clamp(0.0, 1.0);
                if row_a > 0.0 {
                    self.span(mx0, mx1, py, c, row_a);
                }
                px = px.max(mx1);
            }
            while px < end {
                self.edge_pixel(px, py, cx, cy, hw, hh, r, c, a);
                px += 1;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn edge_pixel(&mut self, px: i32, py: i32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32, c: Rgb, a: f32) {
        let d = sd_round_rect(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r);
        let cov = (0.5 - d).clamp(0.0, 1.0);
        if cov > 0.0 {
            self.blend(px, py, c, a * cov);
        }
    }

    /// Source-over of one colour onto the pixels `x0..x1` of row `y` (already clipped to the canvas), in 8.8 fixed
    /// point and without per-pixel bounds checks.
    fn span(&mut self, x0: i32, x1: i32, y: i32, c: Rgb, a: f32) {
        let row = y as usize * self.w;
        let pixels = &mut self.data[(row + x0 as usize) * 4..(row + x1 as usize) * 4];
        if a >= 0.999 {
            let px = [c[2], c[1], c[0], 255];
            for p in pixels.chunks_exact_mut(4) {
                p.copy_from_slice(&px);
            }
            return;
        }
        let ia = (a.clamp(0.0, 1.0) * 256.0) as u32;
        let inv = 256 - ia;
        let src = [c[2] as u32 * ia, c[1] as u32 * ia, c[0] as u32 * ia, 255 * ia];
        for p in pixels.chunks_exact_mut(4) {
            for k in 0..4 {
                p[k] = ((src[k] + p[k] as u32 * inv) >> 8) as u8;
            }
        }
    }

    /// Anti-aliased outline of width `t`, drawn inside the shape.
    pub fn stroke_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, t: f32, c: Rgb, a: f32) {
        let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let r = r.min(hw).min(hh);
        for py in (y.floor() as i32).max(0)..((y + h).ceil() as i32).min(self.h as i32) {
            for px in (x.floor() as i32).max(0)..((x + w).ceil() as i32).min(self.w as i32) {
                let d = sd_round_rect(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r);
                let cov = (0.5 - (d + t / 2.0).abs() + t / 2.0).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, c, a * cov);
                }
            }
        }
    }

    /// Rounded rectangle with a vertical gradient from `top` to `bottom`.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_round_rect_grad(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, top: Rgb, bottom: Rgb, a: f32) {
        let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let r = r.min(hw).min(hh);
        let clip = self.clip;
        for py in (y.floor() as i32).max(clip[1])..((y + h).ceil() as i32).min(clip[3]) {
            let t = ((py as f32 + 0.5 - y) / h.max(1.0)).clamp(0.0, 1.0);
            let c = [0, 1, 2].map(|i| (top[i] as f32 + (bottom[i] as f32 - top[i] as f32) * t + 0.5) as u8);
            for px in (x.floor() as i32).max(clip[0])..((x + w).ceil() as i32).min(clip[2]) {
                let d = sd_round_rect(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, c, a * cov);
                }
            }
        }
    }

    /// Paint an R8 coverage bitmap (a line of text) with colour `c`, its top-left at (x, y).
    pub fn blit_coverage(&mut self, x: i32, y: i32, w: usize, h: usize, cov: &[u8], c: Rgb, a: f32) {
        let ia = (a.clamp(0.0, 1.0) * 256.0) as u32;
        if ia == 0 {
            return;
        }
        let (cr, cg, cb) = (c[0] as u32, c[1] as u32, c[2] as u32);
        let clip = self.clip;
        for row in 0..h {
            let dy = y + row as i32;
            if dy < clip[1] || dy >= clip[3] {
                continue;
            }
            for col in 0..w {
                let k = cov[row * w + col] as u32;
                if k == 0 {
                    continue;
                }
                let dx = x + col as i32;
                if dx < clip[0] || dx >= clip[2] {
                    continue;
                }
                let a8 = ((k * ia) >> 8).min(255);
                let inv = 255 - a8;
                let o = (dy as usize * self.w + dx as usize) * 4;
                let d = &mut self.data[o..o + 4];
                d[0] = ((cb * a8 + d[0] as u32 * inv + 127) / 255) as u8;
                d[1] = ((cg * a8 + d[1] as u32 * inv + 127) / 255) as u8;
                d[2] = ((cr * a8 + d[2] as u32 * inv + 127) / 255) as u8;
                d[3] = ((255 * a8 + d[3] as u32 * inv + 127) / 255) as u8;
            }
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, a: f32) {
        self.fill_round_rect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, c, a);
    }

    /// Blit a premultiplied RGBA8 bitmap (e.g. a rasterised Lucide icon) with its top-left at (x, y).
    pub fn blit_premul(&mut self, x: i32, y: i32, w: usize, h: usize, data: &[u8], alpha: f32) {
        for row in 0..h {
            for col in 0..w {
                let i = (row * w + col) * 4;
                let sa = data[i + 3] as f32 / 255.0 * alpha;
                if sa <= 0.0 {
                    continue;
                }
                let (dx, dy) = (x + col as i32, y + row as i32);
                if dx < self.clip[0] || dy < self.clip[1] || dx >= self.clip[2] || dy >= self.clip[3] {
                    continue;
                }
                let o = (dy as usize * self.w + dx as usize) * 4;
                let inv = 1.0 - sa;
                let d = &mut self.data[o..o + 4];
                d[0] = (data[i + 2] as f32 * alpha + d[0] as f32 * inv + 0.5).min(255.0) as u8;
                d[1] = (data[i + 1] as f32 * alpha + d[1] as f32 * inv + 0.5).min(255.0) as u8;
                d[2] = (data[i] as f32 * alpha + d[2] as f32 * inv + 0.5).min(255.0) as u8;
                d[3] = (sa * 255.0 + d[3] as f32 * inv + 0.5).min(255.0) as u8;
            }
        }
    }

    /// Anti-aliased line segment with round caps.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, c: Rgb, a: f32) {
        let (minx, maxx) = (x0.min(x1) - t, x0.max(x1) + t);
        let (miny, maxy) = (y0.min(y1) - t, y0.max(y1) + t);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for py in (miny.floor() as i32).max(0)..(maxy.ceil() as i32).min(self.h as i32) {
            for px in (minx.floor() as i32).max(0)..(maxx.ceil() as i32).min(self.w as i32) {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let h = (((fx - x0) * dx + (fy - y0) * dy) / len2).clamp(0.0, 1.0);
                let d = (fx - (x0 + dx * h)).hypot(fy - (y0 + dy * h)) - t / 2.0;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, c, a * cov);
                }
            }
        }
    }

    /// Liquid glass over what is already painted: the pixels under the rounded rectangle are blurred, bent at the
    /// rim like a lens, tinted and given a lit edge. `scale` is the HiDPI factor of the canvas.
    ///
    /// It is cheap because the blur never sees the full picture: the backdrop is sampled sparsely into a copy 4x smaller
    /// in each direction, blurred there, and read back with bilinear filtering (a blur hides the loss). The lens maths
    /// runs only in the band along the edge; the interior is one filtered read and one blend per pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn glass_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, dark: bool, strength: f32, scale: f32) {
        const F: usize = 4;
        let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let r = r.min(hw).min(hh);
        let clip = self.clip;
        let (bx0, by0) = ((x.floor() as i32).max(clip[0]), (y.floor() as i32).max(clip[1]));
        let (bx1, by1) = (((x + w).ceil() as i32).min(clip[2]), ((y + h).ceil() as i32).min(clip[3]));
        if bx0 >= bx1 || by0 >= by1 {
            return;
        }
        // the backdrop the blur reads: the glass plus a margin, clamped to the canvas
        let m = (10.0 * scale).ceil() as i32;
        let (rx0, ry0) = ((x.floor() as i32 - m).max(0), (y.floor() as i32 - m).max(0));
        let (rx1, ry1) = (((x + w).ceil() as i32 + m).min(self.w as i32), ((y + h).ceil() as i32 + m).min(self.h as i32));
        // the backdrop at full resolution, before anything is drawn over it: the lens at the rim bends this, so the
        // deformation of the picture near the edge and the corners is sharp while the middle stays frosted
        let (rw, rh) = ((rx1 - rx0) as usize, (ry1 - ry0) as usize);
        let mut sharp = Vec::with_capacity(rw * rh * 4);
        for yy in ry0..ry1 {
            let o = (yy as usize * self.w + rx0 as usize) * 4;
            sharp.extend_from_slice(&self.data[o..o + rw * 4]);
        }
        let (lw, lh) = (((rx1 - rx0) as usize).div_ceil(F).max(1), ((ry1 - ry0) as usize).div_ceil(F).max(1));
        let mut lo = vec![[0f32; 4]; lw * lh];
        for ly in 0..lh {
            for lx in 0..lw {
                // eight taps of the 4x4 block (a checker): less shimmer when the picture under the glass moves
                let mut acc = [0f32; 4];
                for (ox, oy) in [(0, 0), (2, 0), (1, 1), (3, 1), (0, 2), (2, 2), (1, 3), (3, 3)] {
                    let sx = (rx0 as usize + lx * F + ox).min(rx1 as usize - 1);
                    let sy = (ry0 as usize + ly * F + oy).min(ry1 as usize - 1);
                    let i = (sy * self.w + sx) * 4;
                    for k in 0..4 {
                        acc[k] += self.data[i + k] as f32;
                    }
                }
                // the buffer is B,G,R,A; the small copy is R,G,B,A
                lo[ly * lw + lx] = [acc[2], acc[1], acc[0], acc[3]].map(|v| v * (1.0 / (8.0 * 255.0)));
            }
        }
        // two box passes in each direction ~ a gaussian of about 11 px
        let mut tmp = vec![[0f32; 4]; lw * lh];
        for _ in 0..2 {
            box_blur(&lo, &mut tmp, lw, lh, 2, true);
            box_blur(&tmp, &mut lo, lw, lh, 2, false);
        }
        let strength = strength.clamp(0.0, 1.5);
        // a pale, thin tint so that the backdrop shows through and stays vivid; in the dark theme the backdrop is also
        // dimmed a little so that white icons read over a bright picture
        let (tint, ta, dim, sat) = if dark { ([0.80f32, 0.86, 0.95], 0.16 * strength, 0.80, 1.55) } else { ([1.0f32, 1.0, 1.0], 0.34 * strength, 1.0, 1.35) };
        let band = (hh.min(hw) * 0.7).min(18.0 * scale).max(1.0);
        let rim_w = (1.5 * scale).max(1.0);
        let bend = band;
        let light = [-0.5f32, -0.866];
        // the light on the edge: a thin bright rim, and under it a soft glow that fades over the lens band (the glass
        // catches more light where it is steep), stronger on the side facing the light and faint on the far side
        let lighting = |nx: f32, ny: f32, depth: f32| -> f32 {
            let lit = nx * light[0] + ny * light[1];
            let facing = lit.max(0.0) + (-lit).max(0.0) * 0.5;
            let mut v = 0.0;
            if depth < rim_w {
                v = (1.0 - depth.max(0.0) / rim_w) * (if dark { 0.30 } else { 0.55 } + facing * if dark { 0.65 } else { 0.5 });
            }
            if depth < band {
                let u = 1.0 - depth.max(0.0) / band;
                v += u * u * u * (0.05 + facing * 0.11) * if dark { 1.0 } else { 0.6 };
            }
            v.min(0.95) * strength
        };
        let (tr, tg, tb) = (tint[0] * ta, tint[1] * ta, tint[2] * ta);
        let keep = 1.0 - ta;
        // the tint and the colour lift are affine, so they are applied to the small copy once, not to every pixel
        let shade = |c: [f32; 4], sheen: f32| -> [f32; 4] {
            let lum = (c[0] + c[1] + c[2]) * (1.0 / 3.0);
            let ga = ta + c[3] * keep;
            let lift = |v: f32, t: f32| (t + (lum + (v - lum) * sat).max(0.0) * dim * keep).min(ga);
            let mut g = [lift(c[0], tr), lift(c[1], tg), lift(c[2], tb)];
            for v in &mut g {
                *v = *v * (1.0 - sheen) + sheen * ga;
            }
            [g[0], g[1], g[2], ga]
        };
        // a gloss over the upper part of the panel, fading downwards
        let sheen_at = |py: f32| 0.16 * strength * (1.0 - (py - y) / (h * 0.62)).clamp(0.0, 1.0);
        for (i, c) in lo.iter_mut().enumerate() {
            *c = shade(*c, sheen_at(ry0 as f32 + ((i / lw) as f32 + 0.5) * F as f32));
        }
        let (data, stride) = (&mut *self.data, self.w);
        let sample = |px: f32, py: f32| -> [f32; 4] {
            let u = ((px - rx0 as f32) / F as f32 - 0.5).clamp(0.0, (lw - 1) as f32);
            let v = ((py - ry0 as f32) / F as f32 - 0.5).clamp(0.0, (lh - 1) as f32);
            let (u0, v0) = (u as usize, v as usize);
            let (u1, v1) = ((u0 + 1).min(lw - 1), (v0 + 1).min(lh - 1));
            let (fu, fv) = (u - u0 as f32, v - v0 as f32);
            let (a, b, c, d) = (lo[v0 * lw + u0], lo[v0 * lw + u1], lo[v1 * lw + u0], lo[v1 * lw + u1]);
            let mut o = [0f32; 4];
            for k in 0..4 {
                o[k] = (a[k] * (1.0 - fu) + b[k] * fu) * (1.0 - fv) + (c[k] * (1.0 - fu) + d[k] * fu) * fv;
            }
            o
        };
        // the rim is thin, so the nearest pixel is enough (and one read instead of four)
        let sample_sharp = |px: f32, py: f32| -> [f32; 4] {
            let u = ((px - rx0 as f32) as usize).min(rw - 1);
            let v = ((py - ry0 as f32) as usize).min(rh - 1);
            let i = (v * rw + u) * 4;
            [sharp[i + 2] as f32 * (1.0 / 255.0), sharp[i + 1] as f32 * (1.0 / 255.0), sharp[i] as f32 * (1.0 / 255.0), sharp[i + 3] as f32 * (1.0 / 255.0)]
        };
        // shade one pixel: read the (bent) blurred backdrop, tint it, light the rim, blend by coverage
        let put = |data: &mut [u8], px: i32, py: i32, sx: f32, sy: f32, rim: f32, cov: f32, w: f32| {
            let mut b = sample(sx, sy);
            if w > 0.0 {
                let sh = shade(sample_sharp(sx, sy), sheen_at(sy));
                for k in 0..4 {
                    b[k] = b[k] * (1.0 - w) + sh[k] * w;
                }
            }
            let ga = b[3];
            let mut g = [b[0], b[1], b[2]];
            if rim > 0.0 {
                for v in &mut g {
                    *v = *v * (1.0 - rim) + rim * ga;
                }
            }
            let i = (py as usize * stride + px as usize) * 4;
            let new = [g[2], g[1], g[0], ga];
            let icov = 1.0 - cov;
            for k in 0..4 {
                let v = (new[k].min(ga).min(1.0)) * 255.0;
                data[i + k] = (data[i + k] as f32 * icov + v * cov + 0.5) as u8;
            }
        };
        // the part of the glass deeper than `band` has no lens and no rim: the same shape pulled in by `band`
        let mut rowbuf = vec![[0f32; 4]; lw];
        let (iw, ih, ir) = (hw - band, hh - band, (r - band).max(0.0));
        // a soft shadow just below the panel, in the ring around it only (the glass covers the rest)
        let sm = (6.0 * scale).ceil() as i32;
        let off = 2.0 * scale;
        if strength > 0.01 {
            let (sx0, sx1) = ((bx0 - sm).max(clip[0]), (bx1 + sm).min(clip[2]));
            let (sy0, sy1) = ((by0 - sm).max(clip[1]), (by1 + sm + off.ceil() as i32).min(clip[3]));
            for py in sy0..sy1 {
                let fy = py as f32 + 0.5;
                let dy = (fy - cy).abs();
                // along a row the glass covers a known stretch; the shadow is only outside it
                let (s0, s1) = if dy < hh {
                    let over = dy - (hh - r);
                    let half = if over > 0.0 { hw - r + (r * r - over * over).max(0.0).sqrt() } else { hw };
                    (((cx - half + 1.0).ceil() as i32).clamp(sx0, sx1), ((cx + half - 1.0).floor() as i32).clamp(sx0, sx1))
                } else {
                    (sx1, sx1)
                };
                for px in (sx0..s0).chain(s1..sx1) {
                    let fx = px as f32 + 0.5;
                    let d = sd_round_rect(fx, fy - off, cx, cy, hw, hh, r);
                    if d >= sm as f32 {
                        continue;
                    }
                    let t = 1.0 - d.max(0.0) / sm as f32;
                    let a = 0.30 * strength * t * t;
                    let i = (py as usize * stride + px as usize) * 4;
                    for k in 0..3 {
                        data[i + k] = (data[i + k] as f32 * (1.0 - a) + 0.5) as u8;
                    }
                    data[i + 3] = (255.0 * a + data[i + 3] as f32 * (1.0 - a) + 0.5) as u8;
                }
            }
        }
        for py in by0..by1 {
            let fy = py as f32 + 0.5;
            let dy = (fy - cy).abs();
            // the interior of this row, [ix0, ix1)
            let (ix0, ix1) = if dy < ih {
                let over = dy - (ih - ir);
                let half = if over > 0.0 { iw - ir + (ir * ir - over * over).max(0.0).sqrt() } else { iw };
                (((cx - half).ceil() as i32).max(bx0), ((cx + half).floor() as i32).min(bx1))
            } else {
                (bx1, bx1)
            };
            let edge = |data: &mut [u8], px: i32| {
                let fx = px as f32 + 0.5;
                let (dx, dyy) = (fx - cx, fy - cy);
                let (qx, qy) = (dx.abs() - hw + r, dyy.abs() - hh + r);
                let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    return;
                }
                let depth = -d;
                // outward normal of the rounded rectangle
                let (nx, ny) = if qx > 0.0 && qy > 0.0 {
                    let l = qx.hypot(qy);
                    (qx / l * dx.signum(), qy / l * dyy.signum())
                } else if qx > qy {
                    (dx.signum(), 0.0)
                } else {
                    (0.0, dyy.signum())
                };
                let (mut sx, mut sy, mut rim, mut w) = (fx, fy, 0.0, 0.0);
                if depth < band {
                    // a lens: near the edge the glass shows what lies further in, squeezed
                    let t = (depth.max(0.0) / band).min(1.0);
                    let k = (1.0 - t) * (1.0 - t) * bend;
                    sx -= nx * k;
                    sy -= ny * k;
                    let u = 1.0 - t;
                    w = ((u - 0.45) / 0.55).max(0.0) * 0.95;
                    rim = lighting(nx, ny, depth);
                }
                put(data, px, py, sx, sy, rim, cov, w);
            };
            if ix0 >= ix1 {
                // a row in the top or bottom band: along its flat stretch the normal is straight up or down, so the lens
                // shift, the rim and the coverage are the same for every pixel and the row is one horizontal lerp
                let half = (hw - r).min(hw - hh + dy).max(0.0);
                let (fx0, fx1) = (((cx - half).ceil() as i32).clamp(bx0, bx1), ((cx + half).floor() as i32).clamp(bx0, bx1));
                let d = dy - hh;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if fx0 >= fx1 || cov <= 0.0 {
                    for px in bx0..bx1 {
                        edge(&mut *data, px);
                    }
                    continue;
                }
                for px in bx0..fx0 {
                    edge(&mut *data, px);
                }
                let depth = -d;
                let ny = if fy < cy { -1.0 } else { 1.0 };
                let (mut sy, mut rim, mut w) = (fy, 0.0, 0.0);
                if depth < band {
                    let t = (depth.max(0.0) / band).min(1.0);
                    sy -= ny * (1.0 - t) * (1.0 - t) * bend;
                    w = ((1.0 - t - 0.45) / 0.55).max(0.0) * 0.95;
                    rim = lighting(0.0, ny, depth);
                }
                let v = ((sy - ry0 as f32) / F as f32 - 0.5).clamp(0.0, (lh - 1) as f32);
                let (v0, fv) = (v as usize, v - v as usize as f32);
                let v1 = (v0 + 1).min(lh - 1);
                for (i, row) in rowbuf.iter_mut().enumerate() {
                    let (a, b) = (lo[v0 * lw + i], lo[v1 * lw + i]);
                    for k in 0..4 {
                        row[k] = a[k] + (b[k] - a[k]) * fv;
                    }
                }
                let icov = 1.0 - cov;
                for px in fx0..fx1 {
                    let u = ((px as f32 + 0.5 - rx0 as f32) / F as f32 - 0.5).clamp(0.0, (lw - 1) as f32);
                    let (u0, fu) = (u as usize, u - u as usize as f32);
                    let (a, b) = (rowbuf[u0], rowbuf[(u0 + 1).min(lw - 1)]);
                    let mut g = [0f32; 4];
                    for k in 0..4 {
                        g[k] = a[k] + (b[k] - a[k]) * fu;
                    }
                    if w > 0.0 {
                        let sh = shade(sample_sharp(px as f32 + 0.5, sy), sheen_at(sy));
                        for k in 0..4 {
                            g[k] = g[k] * (1.0 - w) + sh[k] * w;
                        }
                    }
                    if rim > 0.0 {
                        let ga = g[3];
                        for v in &mut g[..3] {
                            *v = *v * (1.0 - rim) + rim * ga;
                        }
                    }
                    let o = (py as usize * stride + px as usize) * 4;
                    let new = [g[2], g[1], g[0], g[3]];
                    let dst = &mut data[o..o + 4];
                    if cov >= 1.0 {
                        for k in 0..4 {
                            dst[k] = (new[k] * 255.0 + 0.5) as u8;
                        }
                    } else {
                        for k in 0..4 {
                            dst[k] = (dst[k] as f32 * icov + new[k] * 255.0 * cov + 0.5) as u8;
                        }
                    }
                }
                for px in fx1..bx1 {
                    edge(&mut *data, px);
                }
                continue;
            }
            for px in bx0..ix0 {
                edge(&mut *data, px);
            }
            // the interior: blend the two cell rows once, then only a horizontal lerp per pixel
            let v = ((fy - ry0 as f32) / F as f32 - 0.5).clamp(0.0, (lh - 1) as f32);
            let (v0, fv) = (v as usize, v - v as usize as f32);
            let v1 = (v0 + 1).min(lh - 1);
            for (i, row) in rowbuf.iter_mut().enumerate() {
                let (a, b) = (lo[v0 * lw + i], lo[v1 * lw + i]);
                for k in 0..4 {
                    row[k] = a[k] + (b[k] - a[k]) * fv;
                }
            }
            for px in ix0..ix1 {
                let u = ((px as f32 + 0.5 - rx0 as f32) / F as f32 - 0.5).clamp(0.0, (lw - 1) as f32);
                let (u0, fu) = (u as usize, u - u as usize as f32);
                let (a, b) = (rowbuf[u0], rowbuf[(u0 + 1).min(lw - 1)]);
                let o = (py as usize * stride + px as usize) * 4;
                let dst = &mut data[o..o + 4];
                dst[0] = ((a[2] + (b[2] - a[2]) * fu) * 255.0 + 0.5) as u8;
                dst[1] = ((a[1] + (b[1] - a[1]) * fu) * 255.0 + 0.5) as u8;
                dst[2] = ((a[0] + (b[0] - a[0]) * fu) * 255.0 + 0.5) as u8;
                dst[3] = ((a[3] + (b[3] - a[3]) * fu) * 255.0 + 0.5) as u8;
            }
            for px in ix1..bx1 {
                edge(&mut *data, px);
            }
        }
    }

    /// Cut the four corners of the whole canvas to a rounded shape (alpha mask).
    pub fn mask_round_corners(&mut self, r: f32) {
        let (w, h) = (self.w as f32, self.h as f32);
        let ri = r.ceil() as usize;
        for &(x0, y0) in &[(0, 0), (self.w.saturating_sub(ri), 0), (0, self.h.saturating_sub(ri)), (self.w.saturating_sub(ri), self.h.saturating_sub(ri))] {
            for y in y0..(y0 + ri).min(self.h) {
                for x in x0..(x0 + ri).min(self.w) {
                    let d = sd_round_rect(x as f32 + 0.5, y as f32 + 0.5, w / 2.0, h / 2.0, w / 2.0, h / 2.0, r);
                    let cov = (0.5 - d).clamp(0.0, 1.0);
                    if cov < 1.0 {
                        let i = (y * self.w + x) * 4;
                        for k in 0..4 {
                            self.data[i + k] = (self.data[i + k] as f32 * cov + 0.5) as u8;
                        }
                    }
                }
            }
        }
    }

    /// Blit a glyph whose pen origin is (x, baseline_y).
    pub fn glyph(&mut self, g: &Glyph, x: i32, baseline_y: i32, c: Rgb, a: f32) {
        if g.width == 0 {
            return;
        }
        let top = baseline_y - g.ymin - g.height as i32;
        for row in 0..g.height {
            for col in 0..g.width {
                let cov = g.bitmap[row * g.width + col];
                if cov != 0 {
                    // slight contrast lift so light-on-dark text does not look thin
                    let k = (cov as f32 / 255.0).powf(0.85);
                    self.blend(x + g.xmin + col as i32, top + row as i32, c, a * k);
                }
            }
        }
    }
}

/// Box blur of `r` cells either side with the edge repeated, along rows (`horizontal`) or columns.
fn box_blur(src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len, step, line_step) = if horizontal { (h, w, 1, w) } else { (w, h, w, 1) };
    let inv = 1.0 / (2 * r + 1) as f32;
    for l in 0..lines {
        let base = l * line_step;
        let at = |i: isize| src[base + i.clamp(0, len as isize - 1) as usize * step];
        let mut sum = [0f32; 4];
        for i in -(r as isize)..=(r as isize) {
            let p = at(i);
            for k in 0..4 {
                sum[k] += p[k];
            }
        }
        for i in 0..len {
            dst[base + i * step] = sum.map(|v| v * inv);
            let (add, sub) = (at((i + r + 1) as isize), at(i as isize - r as isize));
            for k in 0..4 {
                sum[k] += add[k] - sub[k];
            }
        }
    }
}

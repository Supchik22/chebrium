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

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

thread_local! {
    /// How much corners are smoothed into a squircle, 0 (circular arcs) .. 1 (see `set_corner_smoothing`). Per thread:
    /// a window is drawn on one thread, and tests running side by side do not see each other's setting.
    static CORNER_SMOOTHING: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// Make every rounded rectangle drawn from now on use "continuous" corners: 0 keeps circular arcs, 1 is a full squircle
/// (the corner curve starts further along the sides and bends gradually, a superellipse). The runtime sets it from the
/// theme (`Appearance::corners`) before each frame.
pub fn set_corner_smoothing(s: f32) {
    CORNER_SMOOTHING.with(|c| c.set(s.clamp(0.0, 1.0)));
}

/// The theme's glass, as the shell writes it in `appearance.toml`: `look` = [blur, refraction, bezel, dispersion, rim,
/// rim sharpness, saturation], the tint (rgb + strength) and the 1px border (rgb + opacity), and whether the theme is
/// dark (the tint belongs to that). Set by the runtime before each frame, like the corner smoothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassLook {
    pub look: [f32; 7],
    pub tint: [f32; 4],
    pub border: [f32; 4],
    pub dark: bool,
}

impl Default for GlassLook {
    fn default() -> GlassLook {
        GlassLook { look: [5.0, 26.0, 24.0, 0.04, 0.85, 1.0, 1.22], tint: [1.0, 1.0, 1.0, 0.12], border: [0.0; 4], dark: false }
    }
}

thread_local! {
    static GLASS_LOOK: std::cell::Cell<GlassLook> = std::cell::Cell::new(GlassLook::default());
}

pub fn set_glass_look(g: GlassLook) {
    GLASS_LOOK.with(|c| c.set(g));
}

pub fn glass_look() -> GlassLook {
    GLASS_LOOK.with(|c| c.get())
}

pub fn corner_smoothing() -> f32 {
    CORNER_SMOOTHING.with(|c| c.get())
}

/// The corner actually drawn for radius `r` on a box of half size `hw` x `hh`: (how far along each side the curve
/// reaches, the superellipse exponent; 2 is a circle). With smoothing the curve reaches further than `r` (so the corner
/// looks about as round as a circular one of radius `r`) and is flatter at its middle. Pills, circles and nearly-pills
/// keep circular ends: a squircle pill looks boxy. The shaders of the shell and of the picture layer do the same.
#[inline]
pub fn corner_shape(r: f32, hw: f32, hh: f32) -> (f32, f32) {
    let s = corner_smoothing();
    let m = hw.min(hh);
    if s <= 0.0 || r < 0.75 || m <= 0.0 {
        return (r.min(m), 2.0);
    }
    // fade the smoothing out as the corner grows to the whole short side
    let k = s * ((0.92 * m - r) / (0.3 * m)).clamp(0.0, 1.0);
    if k <= 0.0 {
        return (r.min(m), 2.0);
    }
    ((r * (1.0 + 0.45 * k)).min(m), 2.0 + 2.6 * k)
}

#[inline]
fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (r, n) = corner_shape(r, hw, hh);
    if n > 2.0 {
        let qx = (px - cx).abs() - hw + r;
        let qy = (py - cy).abs() - hh + r;
        return if qx > 0.0 && qy > 0.0 { (qx.powf(n) + qy.powf(n)).powf(1.0 / n) - r } else { qx.max(qy) - r };
    }
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    // Only inside a corner's square are both positive and the distance is a real hypotenuse. Everywhere else the
    // textbook formula `max(q, 0).hypot(..) + min(max(qx, qy), 0) - r` reduces exactly to `max(qx, qy) - r`
    // (hypot of a value and zero is that value, adding zero changes nothing), so the same result costs no hypot call.
    if qx > 0.0 && qy > 0.0 {
        qx.hypot(qy) - r
    } else {
        qx.max(qy) - r
    }
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
        self.clear_rect_to(r, clear_pixel(c, a));
    }

    /// Set the box `r` (x0, y0, x1, y1) to one premultiplied B,G,R,A pixel.
    pub(crate) fn clear_rect_to(&mut self, r: [i32; 4], px: [u8; 4]) {
        let (x0, x1) = (r[0].max(0) as usize, (r[2].max(0) as usize).min(self.w));
        for y in (r[1].max(0) as usize)..(r[3].max(0) as usize).min(self.h) {
            fill_pixels(&mut self.data[(y * self.w + x0) * 4..(y * self.w + x1.max(x0)) * 4], px);
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
        fill_pixels(self.data, px);
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
        let m = corner_shape(r, hw, hh).0.max(1.0);
        let clip = self.clip;
        let mx0 = ((cx - hw + m).ceil() as i32).max(clip[0]);
        let mx1 = ((cx + hw - m).floor() as i32).min(clip[2]);
        let start = (x.floor() as i32).max(clip[0]);
        let end = ((x + w).ceil() as i32).min(clip[2]);
        for py in (y.floor() as i32).max(clip[1])..((y + h).ceil() as i32).min(clip[3]) {
            let mut px = start;
            if mx0 < mx1 {
                // the part before the span, the span, then the part after it
                self.edge_run(px, mx0.min(end), py, cx, cy, hw, hh, r, c, a);
                let d = sd_round_rect(mx0 as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r);
                let row_a = a * (0.5 - d).clamp(0.0, 1.0);
                if row_a > 0.0 {
                    self.span(mx0, mx1, py, c, row_a);
                }
                px = px.max(mx1);
            }
            self.edge_run(px, end, py, cx, cy, hw, hh, r, c, a);
        }
    }

    /// The pixels `x0..x1` of row `py` outside the flat span: each through `edge_pixel`, except those the row's
    /// geometry shows to be surely inside the shape (full coverage: blended as they are, no distance needed) or surely
    /// outside it (skipped). A round button is nearly all such pixels, and the distance (a `hypot` each) was most of
    /// its cost. The margin keeps every pixel near the outline on the exact path, so the bytes do not change.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn edge_run(&mut self, x0: i32, x1: i32, py: i32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32, c: Rgb, a: f32) {
        if x0 >= x1 {
            return;
        }
        let (inner, outer) = shape_row_limits(py as f32 + 0.5, cy, hw, hh, r);
        // pixels with full coverage come in one stretch: they are blended together (`blend_span_float` does what
        // `blend` does to each, byte for byte)
        let row = py as usize * self.w;
        let mut run: Option<i32> = None;
        for px in x0..x1 {
            let ax = (px as f32 + 0.5 - cx).abs();
            if ax <= inner && ax < outer {
                run.get_or_insert(px);
                continue;
            }
            if let Some(from) = run.take() {
                blend_span_float(&mut self.data[(row + from as usize) * 4..(row + px as usize) * 4], c, a);
            }
            if ax < outer {
                self.edge_pixel(px, py, cx, cy, hw, hh, r, c, a);
            }
        }
        if let Some(from) = run {
            blend_span_float(&mut self.data[(row + from as usize) * 4..(row + x1 as usize) * 4], c, a);
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
            fill_pixels(pixels, [c[2], c[1], c[0], 255]);
            return;
        }
        let ia = (a.clamp(0.0, 1.0) * 256.0) as u32;
        blend_span(pixels, [c[2] as u32 * ia, c[1] as u32 * ia, c[0] as u32 * ia, 255 * ia], (256 - ia) as u16);
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
        // as in `fill_round_rect`: between the corner zones the coverage is the same along a row, and so is the colour
        // (the gradient is vertical), so that stretch is blended with one alpha; the result is the same as pixel by pixel
        let m = corner_shape(r, hw, hh).0.max(1.0);
        let mx0 = ((cx - hw + m).ceil() as i32).max(clip[0]);
        let mx1 = ((cx + hw - m).floor() as i32).min(clip[2]);
        let start = (x.floor() as i32).max(clip[0]);
        let end = ((x + w).ceil() as i32).min(clip[2]);
        for py in (y.floor() as i32).max(clip[1])..((y + h).ceil() as i32).min(clip[3]) {
            let t = ((py as f32 + 0.5 - y) / h.max(1.0)).clamp(0.0, 1.0);
            let c = [0, 1, 2].map(|i| (top[i] as f32 + (bottom[i] as f32 - top[i] as f32) * t + 0.5) as u8);
            let mut px = start;
            if mx0 < mx1 {
                self.edge_run(px, mx0.min(end), py, cx, cy, hw, hh, r, c, a);
                let d = sd_round_rect(mx0 as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let row = py as usize * self.w;
                    blend_span_float(&mut self.data[(row + mx0 as usize) * 4..(row + mx1 as usize) * 4], c, a * cov);
                }
                px = px.max(mx1);
            }
            self.edge_run(px, end, py, cx, cy, hw, hh, r, c, a);
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
        // the columns inside the clip, worked out once instead of tested pixel by pixel
        let (c0, c1) = ((clip[0] - x).clamp(0, w as i32) as usize, (clip[2] - x).clamp(0, w as i32) as usize);
        if c0 >= c1 {
            return;
        }
        for row in 0..h {
            let dy = y + row as i32;
            if dy < clip[1] || dy >= clip[3] {
                continue;
            }
            let src = &cov[row * w + c0..row * w + c1];
            let o = (dy as usize * self.w + (x + c0 as i32) as usize) * 4;
            let dst = &mut self.data[o..o + (c1 - c0) * 4];
            for (&k, d) in src.iter().zip(dst.chunks_exact_mut(4)) {
                if k == 0 {
                    continue;
                }
                let a8 = ((k as u32 * ia) >> 8).min(255);
                let inv = 255 - a8;
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
        // the rows and columns inside the clip, worked out once
        let clip = self.clip;
        let (c0, c1) = ((clip[0] - x).clamp(0, w as i32) as usize, (clip[2] - x).clamp(0, w as i32) as usize);
        let (r0, r1) = ((clip[1] - y).clamp(0, h as i32) as usize, (clip[3] - y).clamp(0, h as i32) as usize);
        for row in r0..r1 {
            for col in c0..c1 {
                let i = (row * w + col) * 4;
                let sa = data[i + 3] as f32 / 255.0 * alpha;
                if sa <= 0.0 {
                    continue;
                }
                let (dx, dy) = (x + col as i32, y + row as i32);
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

    /// Only the soft shadow of a glass panel, in the ring around it (the panel itself is left as it is). Glass over a
    /// picture the graphics card draws is made there (`layer.rs`): the pixels of the picture are not on this canvas, and
    /// the panel must stay see-through for the card's version to show.
    pub fn glass_shadow(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, strength: f32, scale: f32) {
        let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let r = r.min(hw).min(hh);
        let clip = self.clip;
        let (bx0, by0) = ((x.floor() as i32).max(clip[0]), (y.floor() as i32).max(clip[1]));
        let (bx1, by1) = (((x + w).ceil() as i32).min(clip[2]), ((y + h).ceil() as i32).min(clip[3]));
        let strength = strength.clamp(0.0, 1.5);
        if bx0 >= bx1 || by0 >= by1 || strength <= 0.01 {
            return;
        }
        let (data, stride) = (&mut *self.data, self.w);
        let sm = (6.0 * scale).ceil() as i32;
        let off = 2.0 * scale;
        let (sx0, sx1) = ((bx0 - sm).max(clip[0]), (bx1 + sm).min(clip[2]));
        let (sy0, sy1) = ((by0 - sm).max(clip[1]), (by1 + sm + off.ceil() as i32).min(clip[3]));
        for py in sy0..sy1 {
            let fy = py as f32 + 0.5;
            let dy = (fy - cy).abs();
            // along a row the glass covers a known stretch; the shadow is only outside it
            let (s0, s1) = if dy < hh {
                let rr = corner_shape(r, hw, hh).0;
                let over = dy - (hh - rr);
                let half = if over > 0.0 { hw - rr + (rr * rr - over * over).max(0.0).sqrt() } else { hw };
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

    /// Liquid glass over what is already painted: the same glass as the shell's (`shade_glass` in its `glass.wgsl`, and
    /// `fs_glass` of the picture layer), with the theme's look (`set_glass_look`): the backdrop bent in a band at the
    /// rim by a lens, with a little colour dispersion, lightly blurred, saturated, tinted, lit from above by a crisp rim,
    /// and a soft shadow under it. `scale` is the HiDPI factor of the canvas.
    ///
    /// The blur reads a copy 2-4x smaller than the backdrop (a blur hides the loss); the lens, the dispersion and the
    /// rim run only in the band along the edge, the interior is one filtered read and one blend per pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn glass_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, dark: bool, strength: f32, scale: f32) {
        let g = glass_look();
        let fade = strength.clamp(0.0, 1.0);
        if fade <= 0.0 || w < 1.0 || h < 1.0 {
            return;
        }
        let sc = scale.max(0.5);
        let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let (r, ex) = corner_shape(r.min(hw).min(hh), hw, hh);
        // the soft shadow round the panel first: the glass then covers its inner part
        self.glass_shadow(x, y, w, h, r, fade, scale);
        let clip = self.clip;
        let (bx0, by0) = ((x.floor() as i32).max(clip[0]), (y.floor() as i32).max(clip[1]));
        let (bx1, by1) = (((x + w).ceil() as i32).min(clip[2]), ((y + h).ceil() as i32).min(clip[3]));
        if bx0 >= bx1 || by0 >= by1 {
            return;
        }
        // the theme's tint is for the theme's own light or dark; the other one gets the liquid default (as `app.rs`)
        let tint = if dark == g.dark { g.tint } else if dark { [0.08, 0.08, 0.09, 0.55] } else { [1.0, 1.0, 1.0, 0.12] };
        let short = 2.0 * hw.min(hh);
        let bezel = (g.look[2] * sc).min(short * 0.45);
        let refr = (g.look[1] * sc).min((bezel * 1.15).min(short * 0.45));
        let br = (g.look[0] * sc * 1.1).max(1.0);
        let disp_k = g.look[3];
        let rim_s = g.look[4];
        let sat = if g.look[6] <= 0.0 { 1.15 } else { g.look[6] };
        // the backdrop the glass reads: the panel plus what the lens and the blur reach, clamped to the canvas
        let m = (refr * (1.0 + disp_k) + 2.0 * br + 4.0).ceil() as i32;
        let (rx0, ry0) = ((x.floor() as i32 - m).max(0), (y.floor() as i32 - m).max(0));
        let (rx1, ry1) = (((x + w).ceil() as i32 + m).min(self.w as i32), ((y + h).ceil() as i32 + m).min(self.h as i32));
        let (rw, rh) = ((rx1 - rx0).max(1) as usize, (ry1 - ry0).max(1) as usize);
        // full resolution copy, premultiplied R,G,B,A in 0..1 (the canvas is B,G,R,A)
        let mut sharp = vec![[0f32; 4]; rw * rh];
        for yy in 0..rh {
            let o = ((ry0 as usize + yy) * self.w + rx0 as usize) * 4;
            for xx in 0..rw {
                let i = o + xx * 4;
                let d = &self.data[i..i + 4];
                sharp[yy * rw + xx] = [d[2] as f32 / 255.0, d[1] as f32 / 255.0, d[0] as f32 / 255.0, d[3] as f32 / 255.0];
            }
        }
        // the blur (about a gaussian of sigma br / 2, as the shell's mip read): a copy F times smaller, box passes there
        let sigma = br * 0.5;
        let f = if sigma >= 7.0 { 4 } else if sigma >= 2.2 { 2 } else { 1 };
        let ls = sigma / f as f32;
        let rb = (((1.5 * ls * ls + 0.25).sqrt() - 0.5).round() as usize).max(1);
        let (lw, lh) = (rw.div_ceil(f).max(1), rh.div_ceil(f).max(1));
        let mut lo = vec![[0f32; 4]; lw * lh];
        for ly in 0..lh {
            for lx in 0..lw {
                let mut acc = [0f32; 4];
                let mut n = 0.0;
                for oy in 0..f {
                    for ox in 0..f {
                        let (sx, sy) = ((lx * f + ox).min(rw - 1), (ly * f + oy).min(rh - 1));
                        let c = sharp[sy * rw + sx];
                        for k in 0..4 {
                            acc[k] += c[k];
                        }
                        n += 1.0;
                    }
                }
                lo[ly * lw + lx] = acc.map(|v| v / n);
            }
        }
        let mut tmp = vec![[0f32; 4]; lw * lh];
        for _ in 0..2 {
            box_blur(&lo, &mut tmp, lw, lh, rb, true);
            box_blur(&tmp, &mut lo, lw, lh, rb, false);
        }
        let blurred = |px: f32, py: f32| -> [f32; 4] {
            let u = ((px - rx0 as f32) / f as f32 - 0.5).clamp(0.0, (lw - 1) as f32);
            let v = ((py - ry0 as f32) / f as f32 - 0.5).clamp(0.0, (lh - 1) as f32);
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
        let crisp = |px: f32, py: f32| -> [f32; 4] {
            let u = ((px - rx0 as f32 - 0.5).clamp(0.0, (rw - 1) as f32)) as usize;
            let v = ((py - ry0 as f32 - 0.5).clamp(0.0, (rh - 1) as f32)) as usize;
            sharp[v * rw + u]
        };
        // the backdrop at a place, its blur taken off by `k` (the rim band shows the bend crisply)
        let read = |px: f32, py: f32, k: f32| -> [f32; 4] {
            let b = blurred(px, py);
            if k <= 0.001 {
                return b;
            }
            let c = crisp(px, py);
            let mut o = [0f32; 4];
            for i in 0..4 {
                o[i] = b[i] + (c[i] - b[i]) * k;
            }
            o
        };
        let light = {
            let l = (0.22f32 * 0.22 + 1.0).sqrt();
            [-0.22 / l, -1.0 / l]
        };
        let smooth = |a: f32, b: f32, x: f32| {
            let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let (data, stride) = (&mut *self.data, self.w);
        // deeper than this nothing but the blur, the colour and the tint is left (the rim is far below one 8-bit step)
        let deep = bezel.max(18.0 * sc);
        for py in by0..by1 {
            let fy = py as f32 + 0.5;
            for px in bx0..bx1 {
                let fx = px as f32 + 0.5;
                let (dx, dy) = (fx - cx, fy - cy);
                let (qx, qy) = (dx.abs() - hw + r, dy.abs() - hh + r);
                let corner = qx > 0.0 && qy > 0.0;
                let d = if corner {
                    if ex > 2.0 { (qx.powf(ex) + qy.powf(ex)).powf(1.0 / ex) - r } else { qx.hypot(qy) - r }
                } else {
                    qx.max(qy) - r
                };
                let cov = (0.5 - d).clamp(0.0, 1.0) * fade;
                if cov <= 0.0 {
                    continue;
                }
                let depth = -d;
                let mut col;
                let (mut nx, mut ny) = (0.0f32, 0.0f32);
                if depth >= deep {
                    col = blurred(fx, fy);
                } else {
                    // outward normal
                    if corner {
                        let (gx, gy) = (qx.powf(ex - 1.0), qy.powf(ex - 1.0));
                        let l = gx.hypot(gy).max(1e-6);
                        (nx, ny) = (gx / l * dx.signum(), gy / l * dy.signum());
                    } else if qx > qy {
                        nx = dx.signum();
                    } else {
                        ny = dy.signum();
                    }
                    let t = depth / bezel.max(1.0);
                    let k = 1.0 - t.clamp(0.0, 1.0);
                    let k2 = if t < 1.0 { k * k } else { 0.0 };
                    let (ox, oy) = (-nx * k2 * refr, -ny * k2 * refr);
                    let unblur = 0.8 * k2;
                    col = read(fx + ox, fy + oy, unblur);
                    if k2 > 0.0001 && disp_k >= 0.001 {
                        col[0] = read(fx + ox * (1.0 + disp_k), fy + oy * (1.0 + disp_k), unblur)[0];
                        col[2] = read(fx + ox * (1.0 - disp_k), fy + oy * (1.0 - disp_k), unblur)[2];
                    }
                }
                // saturation and tint, in premultiplied colour: the tint is opaque, so it also fills in the alpha
                let a0 = col[3];
                let lum = col[0] * 0.299 + col[1] * 0.587 + col[2] * 0.114;
                for k in 0..3 {
                    col[k] = (lum + (col[k] - lum) * sat).max(0.0);
                }
                let ta = (tint[3] * strength.max(1.0)).min(0.92);
                let a = a0 * (1.0 - ta) + ta;
                for k in 0..3 {
                    col[k] = col[k] * (1.0 - ta) + tint[k] * ta + 0.015 * a;
                }
                col[3] = a;
                if depth < deep {
                    let dl = depth / sc;
                    if dl < 40.0 {
                        let nd = nx * light[0] + ny * light[1];
                        let edge = (-dl / 0.6).exp();
                        let inner = (-dl / 3.2).exp();
                        let top = nd.max(0.0).powf(2.2);
                        let bottom = (-nd).max(0.0).powf(3.0) * 0.42;
                        let l = if a > 0.0 { (col[0] * 0.299 + col[1] * 0.587 + col[2] * 0.114) / a } else { 0.0 };
                        let rim_k = 1.0 + (0.45 - 1.0) * smooth(0.45, 0.9, l);
                        let rim = rim_k * (edge * (0.10 + 0.85 * (top + bottom)) + inner * 0.10 * top) * rim_s;
                        // light is added: it also makes the glass a little more solid there
                        let na = (col[3] + rim).min(1.0);
                        for k in 0..3 {
                            col[k] += rim;
                        }
                        col[3] = na;
                    }
                    // a 1px inner border in the theme's border colour
                    let band = (1.0 - smooth(0.0, 1.5, dl)) * g.border[3];
                    if band > 0.0 {
                        for k in 0..3 {
                            col[k] = col[k] * (1.0 - band) + g.border[k] * band;
                        }
                        col[3] = col[3] * (1.0 - band) + band;
                    }
                }
                let i = (py as usize * stride + px as usize) * 4;
                let ga = col[3].min(1.0);
                let new = [col[2].min(ga), col[1].min(ga), col[0].min(ga), ga];
                let icov = 1.0 - cov;
                for k in 0..4 {
                    data[i + k] = (data[i + k] as f32 * icov + new[k] * 255.0 * cov + 0.5) as u8;
                }
            }
        }
    }

    /// Cut the four corners of the whole canvas to a rounded shape (alpha mask).
    pub fn mask_round_corners(&mut self, r: f32) {
        let (w, h) = (self.w as f32, self.h as f32);
        let ri = corner_shape(r, w / 2.0, h / 2.0).0.ceil() as usize;
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

/// The part of the box `area` (x0, y0, x1, y1) where `Canvas::fill_round_rect` paints every pixel with one alpha: the
/// columns `x0..x1` returned, over all of the area's rows, away from the shape's rounded ends, with the same coverage on
/// every row. `None` when there is no such part (the area reaches past the shape's top or bottom, or into a corner).
///
/// What for: a repaint of a damaged box clears it to the background and then covers it with a translucent pane (the
/// content pane under a list being scrolled). Where the pane is flat, the result is one colour, worked out once (see
/// `flat_pixel`) and cleared straight in: the pixels are written once instead of once per pane, with the same bytes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn flat_part(x: f32, y: f32, w: f32, h: f32, r: f32, a: f32, area: [i32; 4]) -> Option<(i32, i32, f32)> {
    let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    let r = r.min(hw).min(hh);
    let m = corner_shape(r, hw, hh).0.max(1.0);
    // the same span as `fill_round_rect` works out, with the area as the clip
    let mx0 = ((cx - hw + m).ceil() as i32).max(area[0]);
    let mx1 = ((cx + hw - m).floor() as i32).min(area[2]);
    if mx0 >= mx1 || (y.floor() as i32) > area[1] || ((y + h).ceil() as i32) < area[3] || area[1] >= area[3] {
        return None;
    }
    // inside the span a row's coverage does not depend on the column, so it is taken where `fill_round_rect` takes it
    let row_a = |py: i32| a * (0.5 - sd_round_rect(mx0 as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r)).clamp(0.0, 1.0);
    let first = row_a(area[1]);
    if (area[1] + 1..area[3]).any(|py| row_a(py).to_bits() != first.to_bits()) {
        return None;
    }
    Some((mx0, mx1, first))
}

/// One pixel of a span `fill_round_rect` paints with alpha `row_a` (from `flat_part`) over `under`, through the very
/// code a row goes through.
pub(crate) fn flat_pixel(c: Rgb, row_a: f32, under: [u8; 4]) -> [u8; 4] {
    let mut px = under;
    if row_a > 0.0 {
        if row_a >= 0.999 {
            fill_pixels(&mut px, [c[2], c[1], c[0], 255]);
        } else {
            let ia = (row_a.clamp(0.0, 1.0) * 256.0) as u32;
            blend_span(&mut px, [c[2] as u32 * ia, c[1] as u32 * ia, c[0] as u32 * ia, 255 * ia], (256 - ia) as u16);
        }
    }
    px
}

/// The pixel `Canvas::clear_rect` writes.
pub(crate) fn clear_pixel(c: Rgb, a: f32) -> [u8; 4] {
    [(c[2] as f32 * a + 0.5) as u8, (c[1] as f32 * a + 0.5) as u8, (c[0] as f32 * a + 0.5) as u8, (a * 255.0 + 0.5) as u8]
}

/// For a row through `fy` (pixel centre) of a rounded rectangle centred at `cy`: how far from the centre column a pixel
/// centre may be and surely have full coverage (`inner`), and from how far it surely has none (`outer`), measured as
/// `|x - cx|`. Worked out from the same distance `sd_round_rect` gives (coverage is 1 where it is at most -0.5 and 0
/// where it is at least 0.5), pulled in by a margin far wider than any rounding, so a pixel near the outline is left
/// to the exact test.
/// `row_limits` for the corner as drawn: a squircle holds the circle of its reach (so that circle's inside is surely
/// covered) and lies inside its box (so beyond the box's side nothing is).
fn shape_row_limits(fy: f32, cy: f32, hw: f32, hh: f32, r: f32) -> (f32, f32) {
    let (rr, n) = corner_shape(r, hw, hh);
    if n <= 2.0 {
        return row_limits(fy, cy, hw, hh, rr);
    }
    let (inner, _) = row_limits(fy, cy, hw, hh, rr);
    let outer = if (fy - cy).abs() >= hh + 0.5 + 0.01 { f32::NEG_INFINITY } else { hw + 0.5 + 0.01 };
    (inner, outer)
}

fn row_limits(fy: f32, cy: f32, hw: f32, hh: f32, r: f32) -> (f32, f32) {
    const MARGIN: f32 = 0.01;
    let qy = (fy - cy).abs() - hh + r;
    let base = hw - r;
    let (lin, lout) = (r - 0.5, r + 0.5);
    let inner = if qy > 0.0 {
        if qy < lin - MARGIN {
            base + (lin * lin - qy * qy).sqrt() - MARGIN
        } else {
            f32::NEG_INFINITY
        }
    } else if qy <= lin - MARGIN {
        base + lin - MARGIN
    } else {
        f32::NEG_INFINITY
    };
    let outer = if qy > 0.0 {
        if qy >= lout + MARGIN {
            // the whole row is above or below the shape
            f32::NEG_INFINITY
        } else {
            base + (lout * lout - qy * qy).max(0.0).sqrt() + MARGIN
        }
    } else {
        base + lout + MARGIN
    };
    (inner, outer)
}

/// Set every pixel of `pixels` (whole B,G,R,A quadruples) to `px`. Writing 64 bytes at a time lets the compiler use wide
/// stores; this is what clearing a damaged box and painting an opaque panel spend their time on.
#[inline]
fn fill_pixels(pixels: &mut [u8], px: [u8; 4]) {
    let mut pattern = [0u8; 64];
    for (i, b) in pattern.iter_mut().enumerate() {
        *b = px[i & 3];
    }
    let mut chunks = pixels.chunks_exact_mut(64);
    for ch in &mut chunks {
        ch.copy_from_slice(&pattern);
    }
    let rest = chunks.into_remainder();
    let n = rest.len();
    rest.copy_from_slice(&pattern[..n]);
}

/// Source-over of one colour onto a run of pixels: every byte becomes `(src + dst * inv) >> 8`, where `src` is the
/// colour already multiplied by the alpha (each at most 255 * 256) and `inv` is 256 minus the alpha (at most 256).
///
/// The sum needs 17 bits, but it can be split into 16-bit pieces with the same result:
/// `(s + t) >> 8 == (s >> 8) + (t >> 8) + (((s & 255) + (t & 255)) >> 8)`. In 16-bit lanes the compiler turns the loop
/// into eight-wide multiplies (plain SSE2), several times faster than byte by byte, and every pixel is bit-identical to
/// the 32-bit formula.
#[inline]
fn blend_span(pixels: &mut [u8], src: [u32; 4], inv: u16) {
    const N: usize = 32;
    let mut hi = [0u16; N];
    let mut lo = [0u16; N];
    for i in 0..N {
        hi[i] = (src[i & 3] >> 8) as u16;
        lo[i] = (src[i & 3] & 255) as u16;
    }
    let one = |d: &mut u8, hi: u16, lo: u16| {
        let t = *d as u16 * inv;
        *d = (hi + (t >> 8) + ((lo + (t & 255)) >> 8)) as u8;
    };
    let mut chunks = pixels.chunks_exact_mut(N);
    for ch in &mut chunks {
        for i in 0..N {
            one(&mut ch[i], hi[i], lo[i]);
        }
    }
    for (i, d) in chunks.into_remainder().iter_mut().enumerate() {
        one(d, hi[i], lo[i]);
    }
}

/// `blend` for a run of pixels that share colour and alpha: the same floating-point steps in the same order, so the bytes
/// are identical, without the per-pixel clip test and index arithmetic.
#[inline]
fn blend_span_float(pixels: &mut [u8], c: Rgb, a: f32) {
    if a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let inv = 1.0 - a;
    let s = [c[2] as f32 * a, c[1] as f32 * a, c[0] as f32 * a, 255.0 * a];
    for p in pixels.chunks_exact_mut(4) {
        for k in 0..4 {
            p[k] = (s[k] + p[k] as f32 * inv + 0.5) as u8;
        }
    }
}

#[cfg(test)]
mod fast_path_tests {
    use super::*;

    /// A backdrop with every kind of byte value, so a rounding difference would show.
    fn noise(w: usize, h: usize) -> Vec<u8> {
        let mut x = 0x1234_5678u32;
        (0..w * h * 4)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect()
    }

    /// The distance as it was written before the fast path.
    fn sd_reference(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
        let qx = (px - cx).abs() - hw + r;
        let qy = (py - cy).abs() - hh + r;
        qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
    }

    #[test]
    fn the_distance_without_hypot_is_the_same_number() {
        for i in 0..4000 {
            let (px, py) = ((i % 97) as f32 * 0.73 - 5.0, (i / 97) as f32 * 1.31 - 3.0);
            for (hw, hh, r) in [(20.0, 10.0, 6.0), (30.5, 30.5, 30.5), (12.0, 40.0, 0.0), (8.25, 3.75, 2.5)] {
                let (a, b) = (sd_round_rect(px, py, 25.0, 20.0, hw, hh, r), sd_reference(px, py, 25.0, 20.0, hw, hh, r));
                assert_eq!(a, b, "{px},{py} {hw},{hh},{r}");
            }
        }
    }

    #[test]
    fn the_wide_span_blend_matches_the_byte_formula() {
        for a in [0.001f32, 0.06, 0.2, 0.5, 0.73, 0.98] {
            let c = [200u8, 31, 117];
            let mut fast = noise(37, 1);
            let mut slow = fast.clone();
            let ia = (a * 256.0) as u32;
            blend_span(&mut fast, [c[2] as u32 * ia, c[1] as u32 * ia, c[0] as u32 * ia, 255 * ia], (256 - ia) as u16);
            let src = [c[2] as u32 * ia, c[1] as u32 * ia, c[0] as u32 * ia, 255 * ia];
            for p in slow.chunks_exact_mut(4) {
                for k in 0..4 {
                    p[k] = ((src[k] + p[k] as u32 * (256 - ia)) >> 8) as u8;
                }
            }
            assert_eq!(fast, slow, "alpha {a}");
        }
    }

    #[test]
    fn the_gradient_with_row_spans_matches_pixel_by_pixel() {
        let (w, h) = (90, 70);
        for (rect, r, a) in [([3.3f32, 4.6, 80.0, 60.0], 12.0f32, 0.8f32), ([10.0, 10.0, 50.0, 20.0], 10.0, 1.0), ([0.5, 0.5, 70.0, 40.0], 0.0, 0.35), ([20.4, 3.1, 55.5, 55.5], 27.75, 0.95)] {
            let mut fast = noise(w, h);
            let mut slow = fast.clone();
            let (top, bottom) = ([250, 20, 90], [10, 200, 140]);
            let mut cv = Canvas::new(&mut fast, w, h);
            cv.set_clip([2, 0, 85, 66]);
            cv.fill_round_rect_grad(rect[0], rect[1], rect[2], rect[3], r, top, bottom, a);
            // the old loop: every pixel through `blend`
            let mut cv = Canvas::new(&mut slow, w, h);
            cv.set_clip([2, 0, 85, 66]);
            let [x, y, rw, rh] = rect;
            let (cx, cy, hw, hh) = (x + rw / 2.0, y + rh / 2.0, rw / 2.0, rh / 2.0);
            let r = r.min(hw).min(hh);
            let clip = cv.clip;
            for py in (y.floor() as i32).max(clip[1])..((y + rh).ceil() as i32).min(clip[3]) {
                let t = ((py as f32 + 0.5 - y) / rh.max(1.0)).clamp(0.0, 1.0);
                let c = [0, 1, 2].map(|i| (top[i] as f32 + (bottom[i] as f32 - top[i] as f32) * t + 0.5) as u8);
                for px in (x.floor() as i32).max(clip[0])..((x + rw).ceil() as i32).min(clip[2]) {
                    let cov = (0.5 - sd_reference(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r)).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        cv.blend(px, py, c, a * cov);
                    }
                }
            }
            assert!(fast == slow, "rect {rect:?} radius {r}");
        }
    }

    /// `fill_round_rect` as it was: every pixel outside the flat span through the full distance.
    fn fill_reference(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgb, a: f32) {
        let (cx, cy, hw, hh) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let r = r.min(hw).min(hh);
        let m = r.max(1.0);
        let clip = cv.clip;
        let mx0 = ((cx - hw + m).ceil() as i32).max(clip[0]);
        let mx1 = ((cx + hw - m).floor() as i32).min(clip[2]);
        let start = (x.floor() as i32).max(clip[0]);
        let end = ((x + w).ceil() as i32).min(clip[2]);
        let edge = |cv: &mut Canvas, px: i32, py: i32| {
            let cov = (0.5 - sd_reference(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r)).clamp(0.0, 1.0);
            if cov > 0.0 {
                cv.blend(px, py, c, a * cov);
            }
        };
        for py in (y.floor() as i32).max(clip[1])..((y + h).ceil() as i32).min(clip[3]) {
            let mut px = start;
            if mx0 < mx1 {
                while px < mx0.min(end) {
                    edge(cv, px, py);
                    px += 1;
                }
                let row_a = a * (0.5 - sd_reference(mx0 as f32 + 0.5, py as f32 + 0.5, cx, cy, hw, hh, r)).clamp(0.0, 1.0);
                if row_a > 0.0 {
                    let ia = (row_a.clamp(0.0, 1.0) * 256.0) as u32;
                    for qx in mx0..mx1 {
                        let o = (py as usize * cv.w + qx as usize) * 4;
                        let src = [c[2] as u32 * ia, c[1] as u32 * ia, c[0] as u32 * ia, 255 * ia];
                        for k in 0..4 {
                            cv.data[o + k] = if row_a >= 0.999 { [c[2], c[1], c[0], 255][k] } else { ((src[k] + cv.data[o + k] as u32 * (256 - ia)) >> 8) as u8 };
                        }
                    }
                }
                px = px.max(mx1);
            }
            while px < end {
                edge(cv, px, py);
                px += 1;
            }
        }
    }

    #[test]
    fn round_shapes_skip_the_distance_and_keep_every_byte() {
        let (w, h) = (120, 90);
        let shapes = [
            ([10.0f32, 10.0, 60.0, 60.0], 30.0f32, 1.0f32),
            ([13.3, 7.7, 61.4, 61.4], 30.7, 0.73),
            ([2.5, 20.25, 100.0, 34.0], 17.0, 0.5),
            ([30.0, 30.0, 20.0, 9.0], 3.0, 0.9),
            ([5.6, 5.6, 80.0, 70.0], 0.0, 0.42),
            ([40.1, 3.9, 7.0, 7.0], 0.4, 1.0),
            ([-20.0, -15.0, 90.0, 70.0], 24.0, 0.66),
        ];
        for (rect, r, a) in shapes {
            for clip in [[0, 0, w as i32, h as i32], [17, 11, 77, 59]] {
                let mut fast = noise(w, h);
                let mut slow = fast.clone();
                let mut cv = Canvas::new(&mut fast, w, h);
                cv.set_clip(clip);
                cv.fill_round_rect(rect[0], rect[1], rect[2], rect[3], r, [30, 140, 250], a);
                let mut cv = Canvas::new(&mut slow, w, h);
                cv.set_clip(clip);
                fill_reference(&mut cv, rect[0], rect[1], rect[2], rect[3], r, [30, 140, 250], a);
                assert!(fast == slow, "rect {rect:?} radius {r} alpha {a} clip {clip:?}");
            }
        }
    }

    #[test]
    fn squircle_corners_fill_more_of_the_corner_and_keep_the_quick_path_exact() {
        let (w, h) = (80usize, 60usize);
        let draw = |smooth: f32| {
            set_corner_smoothing(smooth);
            let mut buf = vec![0u8; w * h * 4];
            Canvas::new(&mut buf, w, h).fill_round_rect(4.0, 4.0, 70.0, 50.0, 14.0, [255, 255, 255], 1.0);
            // the same shape pixel by pixel through the distance alone
            let mut slow = vec![0u8; w * h * 4];
            {
                let mut c = Canvas::new(&mut slow, w, h);
                for y in 0..h as i32 {
                    for x in 0..w as i32 {
                        let d = sd_round_rect(x as f32 + 0.5, y as f32 + 0.5, 39.0, 29.0, 35.0, 25.0, 14.0);
                        let cov = (0.5 - d).clamp(0.0, 1.0);
                        if cov > 0.0 {
                            c.blend(x, y, [255, 255, 255], cov);
                        }
                    }
                }
            }
            set_corner_smoothing(0.0);
            (buf, slow)
        };
        let (round, round_slow) = draw(0.0);
        let (sq, sq_slow) = draw(1.0);
        let diff = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap_or(0);
        assert!(diff(&round, &round_slow) <= 1 && diff(&sq, &sq_slow) <= 1, "{} {}", diff(&round, &round_slow), diff(&sq, &sq_slow));
        assert_ne!(round, sq);
        // at the corner's diagonal the squircle bends gradually: a pixel just inside the circle's arc is still covered
        // and the very corner stays empty either way
        let alpha = |b: &[u8], x: usize, y: usize| b[(y * w + x) * 4 + 3];
        assert_eq!(alpha(&sq, 4, 4), 0);
        assert_eq!(alpha(&round, 4, 4), 0);
        // the sides stay straight a shorter way: the squircle's curve reaches further along them
        assert!(alpha(&sq, 4, 4 + 15) < alpha(&round, 4, 4 + 15), "{} vs {}", alpha(&sq, 4, 19), alpha(&round, 4, 19));
        // a pill keeps round ends
        set_corner_smoothing(1.0);
        assert_eq!(corner_shape(10.0, 40.0, 10.0).1, 2.0);
        set_corner_smoothing(0.0);
    }

    #[test]
    fn coverage_blit_respects_the_clip_on_both_sides() {
        let (w, h) = (40, 20);
        let cov: Vec<u8> = (0..30 * 10).map(|i| (i * 7 % 256) as u8).collect();
        let mut buf = noise(w, h);
        let before = buf.clone();
        let mut cv = Canvas::new(&mut buf, w, h);
        cv.set_clip([10, 5, 25, 12]);
        cv.blit_coverage(-3, 2, 30, 10, &cov, [255, 255, 255], 1.0);
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                let inside = (10..25).contains(&x) && (5..12).contains(&y);
                if !inside {
                    assert_eq!(buf[o..o + 4], before[o..o + 4], "{x},{y} is outside the clip");
                }
            }
        }
    }
}

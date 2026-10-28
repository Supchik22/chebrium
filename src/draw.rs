//! What a widget needs from a painter. Rectangles are `[x, y, w, h]` in logical pixels, colours straight RGB
//! (0..1) and `alpha` 0..1.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub trait Draw {
    /// A rounded rectangle in one colour.
    fn fill(&mut self, r: [f32; 4], radius: f32, c: [f32; 3], alpha: f32);
    /// A rounded rectangle with a vertical gradient.
    fn grad(&mut self, r: [f32; 4], radius: f32, top: [f32; 3], bottom: [f32; 3], alpha: f32);
    /// A Lucide icon centred on (`cx`, `cy`), `size` px square.
    fn glyph(&mut self, cx: f32, cy: f32, size: f32, icon: &str, c: [f32; 3], alpha: f32);
    /// A line of text; `y` is the baseline and `x` the left, centre or right edge per `align`.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, x: f32, y: f32, text: &str, size: f32, bold: bool, c: [f32; 3], alpha: f32, align: Align);
    /// Width of a line of text. Painters that cannot measure give a rough guess.
    fn measure(&mut self, text: &str, size: f32, _bold: bool) -> f32 {
        text.chars().count() as f32 * size * 0.55
    }
    /// Limit everything drawn from now on to `r` (inside the clip already in force) until the matching `pop_clip`: a list
    /// that scrolls under a translucent bar, a panel whose contents must not spill out.
    fn push_clip(&mut self, _r: [f32; 4]) {}
    fn pop_clip(&mut self) {}
    /// Where the next thing drawn will go (for `cull`): painters that keep a list of what was drawn say how long it is.
    fn mark(&self) -> (usize, usize) {
        (0, 0)
    }
    /// A painter without real clipping drops what was drawn since `from` and is not wholly between `y0` and `y1`
    /// (a scrolled page), and says where the lowest edge of everything drawn since `from` was.
    fn cull(&mut self, _from: (usize, usize), _y0: f32, _y1: f32) -> f32 {
        0.0
    }
    /// A decoded picture or a video frame: the part `src` (x, y, w, h in picture pixels) of it, scaled into `r`.
    fn picture(&mut self, _r: [f32; 4], _pic: &std::sync::Arc<crate::picture::Pic>, _src: [f32; 4], _alpha: f32) {}
    /// A picture the backend holds under `handle` (a thumbnail in the shell's atlas). Painters without pictures
    /// leave it out.
    fn image(&mut self, _r: [f32; 4], _radius: f32, _handle: u32) {}
    /// A panel of glass for what floats over content (a control pill, a tooltip). A painter that can read what is
    /// behind it blurs and bends that; the default is a flat translucent plate: the tint, a faint rim and a bright edge
    /// along the top.
    fn glass(&mut self, r: [f32; 4], radius: f32, strength: f32, dark: bool) {
        let base = if dark { [0.11, 0.11, 0.11] } else { [0.97, 0.98, 1.0] };
        self.fill([r[0] - 1.0, r[1] - 1.0, r[2] + 2.0, r[3] + 2.0], radius + 1.0, [1.0; 3], 0.10 * strength);
        self.fill(r, radius, base, 0.74 * strength);
        self.fill([r[0] + radius * 0.7, r[1] + 0.5, (r[2] - radius * 1.4).max(0.0), 1.0], 0.5, [1.0; 3], if dark { 0.20 } else { 0.9 } * strength);
    }
}

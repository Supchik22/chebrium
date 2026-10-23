//! A small glyph engine: `ttf-parser` reads the font file where it is and only the outline of a glyph that is
//! actually drawn is rasterised (with `tiny-skia`), then kept by the caller's glyph cache.
//!
//! Why not a font crate that parses everything up front: a Nerd Font has ten thousand glyphs and such a crate holds
//! every outline in memory, about a hundred megabytes for one terminal font. Here a font costs the size of its file.
//! The API follows the part of `fontdue` the programs used, so swapping was a change of names.

use resvg::tiny_skia;
use std::sync::Arc;
use ttf_parser::{Face, OutlineBuilder};

/// Where a rasterised glyph sits and how far the pen moves after it. `xmin` and `ymin` are from the pen origin, y up,
/// like `fontdue`'s.
#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    pub xmin: i32,
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub advance_width: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

/// A font file, shared and never copied. The bytes live as long as the program (fonts are loaded once).
#[derive(Clone)]
pub struct Font {
    face: Arc<Face<'static>>,
}

impl Font {
    /// A font that is part of the program (`include_bytes!`).
    pub fn from_static(bytes: &'static [u8]) -> Option<Font> {
        Face::parse(bytes, 0).ok().map(|f| Font { face: Arc::new(f) })
    }

    /// A font read from a file. The bytes are kept for the rest of the run: a font is loaded once and used until exit.
    pub fn from_vec(bytes: Vec<u8>) -> Option<Font> {
        Font::from_static(Box::leak(bytes.into_boxed_slice()))
    }

    /// The glyph index of `c`; 0 when the font does not have it.
    pub fn lookup_glyph_index(&self, c: char) -> u16 {
        self.face.glyph_index(c).map_or(0, |g| g.0)
    }

    fn scale(&self, px: f32) -> f32 {
        px / self.face.units_per_em().max(1) as f32
    }

    /// How far the pen moves after `c`. Only the advance table is read: the box in `metrics` needs the glyph's whole
    /// outline parsed, which made measuring text the main cost of building a frame.
    pub fn advance(&self, c: char, px: f32) -> f32 {
        self.face.glyph_index(c).and_then(|id| self.face.glyph_hor_advance(id)).map_or(0.0, |a| a as f32 * self.scale(px))
    }

    pub fn metrics(&self, c: char, px: f32) -> Metrics {
        self.measure(c, px).0
    }

    /// Metrics and the pixel box of the glyph's outline: (metrics, left, top) in pixels, y down from the baseline.
    fn measure(&self, c: char, px: f32) -> (Metrics, i32, i32) {
        let k = self.scale(px);
        let Some(id) = self.face.glyph_index(c) else { return (Metrics::default(), 0, 0) };
        let advance_width = self.face.glyph_hor_advance(id).map_or(0.0, |a| a as f32 * k);
        let Some(b) = self.face.glyph_bounding_box(id) else { return (Metrics { advance_width, ..Default::default() }, 0, 0) };
        let (x0, x1) = ((b.x_min as f32 * k).floor() as i32, (b.x_max as f32 * k).ceil() as i32);
        let (y0, y1) = ((b.y_min as f32 * k).floor() as i32, (b.y_max as f32 * k).ceil() as i32);
        (Metrics { xmin: x0, ymin: y0, width: (x1 - x0).max(0) as usize, height: (y1 - y0).max(0) as usize, advance_width }, x0, y1)
    }

    pub fn horizontal_line_metrics(&self, px: f32) -> Option<LineMetrics> {
        let k = self.scale(px);
        Some(LineMetrics { ascent: self.face.ascender() as f32 * k, descent: self.face.descender() as f32 * k, line_gap: self.face.line_gap() as f32 * k })
    }

    /// The glyph as 8-bit coverage (`width` x `height`, top row first) with its metrics.
    pub fn rasterize(&self, c: char, px: f32) -> (Metrics, Vec<u8>) {
        let (m, left, top) = self.measure(c, px);
        if m.width == 0 || m.height == 0 {
            return (m, Vec::new());
        }
        let Some(id) = self.face.glyph_index(c) else { return (m, Vec::new()) };
        let k = self.scale(px);
        let mut path = Path { pb: tiny_skia::PathBuilder::new(), k, left: left as f32, top: top as f32 };
        if self.face.outline_glyph(id, &mut path).is_none() {
            return (m, Vec::new());
        }
        let (Some(path), Some(mut pix)) = (path.pb.finish(), tiny_skia::Pixmap::new(m.width as u32, m.height as u32)) else { return (m, vec![0; m.width * m.height]) };
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(255, 255, 255, 255);
        paint.anti_alias = true;
        pix.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        let coverage = pix.data().chunks_exact(4).map(|p| p[3]).collect();
        (m, coverage)
    }
}

/// Outline callbacks: font units, y up, into pixels, y down.
struct Path {
    pb: tiny_skia::PathBuilder,
    k: f32,
    left: f32,
    top: f32,
}

impl Path {
    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.k - self.left, self.top - y * self.k)
    }
}

impl OutlineBuilder for Path {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.p(x, y);
        self.pb.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.p(x, y);
        self.pb.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let ((x1, y1), (x, y)) = (self.p(x1, y1), self.p(x, y));
        self.pb.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let ((x1, y1), (x2, y2), (x, y)) = (self.p(x1, y1), self.p(x2, y2), self.p(x, y));
        self.pb.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inter() -> Font {
        Font::from_static(crate::text::INTER_REGULAR).expect("the bundled font parses")
    }

    #[test]
    fn advance_is_the_advance_of_the_metrics() {
        let f = inter();
        for c in "Hello, Світ! 0123 Wq".chars() {
            assert_eq!(f.advance(c, 13.0), f.metrics(c, 13.0).advance_width);
        }
    }

    #[test]
    fn a_glyph_has_size_position_and_ink() {
        let f = inter();
        let (m, cov) = f.rasterize('H', 40.0);
        assert!(m.width > 15 && m.height > 20, "{m:?}");
        assert_eq!(cov.len(), m.width * m.height);
        assert!(cov.iter().filter(|&&c| c > 200).count() > 80, "the strokes are solid");
        assert!(cov.iter().any(|&c| c == 0), "and there is space around them");
        assert!(m.advance_width > 20.0);
        assert!(m.ymin >= -1, "a capital sits on the baseline: {m:?}");
    }

    #[test]
    fn a_missing_character_is_glyph_zero_and_a_space_has_no_ink() {
        let f = inter();
        assert_eq!(f.lookup_glyph_index('\u{10FFFF}'), 0);
        assert_ne!(f.lookup_glyph_index('a'), 0);
        let (m, cov) = f.rasterize(' ', 20.0);
        assert!(cov.is_empty() && m.advance_width > 2.0);
    }

    #[test]
    fn the_text_scales_with_the_size() {
        let f = inter();
        let (small, big) = (f.metrics('M', 10.0).advance_width, f.metrics('M', 40.0).advance_width);
        assert!((big / small - 4.0).abs() < 0.1);
        let lm = f.horizontal_line_metrics(20.0).unwrap();
        assert!(lm.ascent > 10.0 && lm.descent < 0.0);
    }
}

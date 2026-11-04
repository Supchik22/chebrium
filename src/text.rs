//! UI text for the shell: a line rasteriser on top of the glyph engine (`glyphs`). The renderer packs the results into a
//! glyph-line atlas and the glass shader draws them as quads (see `Label` in renderer.rs).

use crate::glyphs::{Font, Metrics};
use std::process::Command;

/// The bundled Inter faces, for programs that lay their own text out (a terminal).
pub const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.otf");
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.otf");

pub struct TextFonts {
    regular: Font,
    bold: Font,
    /// Rasterised glyphs by (character, pixel size, weight). A glyph's bitmap does not depend on where on the line it
    /// sits, so a new line (a file name scrolled into view, a clock's next second) is put together from glyphs already
    /// drawn instead of filling every outline again. A mutex rather than a cell: the shell's renderer keeps its fonts
    /// in a struct that must stay shareable.
    glyphs: std::sync::Mutex<std::collections::HashMap<u64, (Metrics, Vec<u8>), std::hash::BuildHasherDefault<crate::soft::Prehashed>>>,
}

/// The glyph cache's key: character, size and weight packed into one number (no two glyphs share one) and spread over
/// all bits by an odd multiplier, so the map needs no hashing of its own.
fn glyph_key(c: char, size: u32, bold: bool) -> u64 {
    ((c as u64) << 33 | (bold as u64) << 32 | size as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// Glyphs kept at most; a program that cycles through many sizes or scripts starts over rather than grow (about a
/// megabyte at UI sizes).
const GLYPHS_KEPT: usize = 4096;

fn load_font(pattern: &str) -> Option<Font> {
    let out = Command::new("fc-match").args(["-f", "%{file}", pattern]).output().ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    let bytes = std::fs::read(path.trim()).ok()?;
    Font::from_vec(bytes)
}

/// One rasterised line: R8 coverage, with the baseline `ascent` pixels below the top.
pub struct Raster {
    pub w: usize,
    pub h: usize,
    pub ascent: f32,
    /// the line's advance width, so it need not be measured again
    pub width: f32,
    pub bitmap: Vec<u8>,
}

impl TextFonts {
    /// Bundled Inter by default; `CHEBRIUM_FONT` picks another family through fontconfig (falling back to "sans").
    pub fn load() -> TextFonts {
        if std::env::var("CHEBRIUM_FONT").is_err() {
            if let (Some(regular), Some(bold)) = (Font::from_static(INTER_REGULAR), Font::from_static(INTER_SEMIBOLD)) {
                return TextFonts { regular, bold, glyphs: Default::default() };
            }
        }
        let name = std::env::var("CHEBRIUM_FONT").unwrap_or_else(|_| "Inter".to_string());
        let regular = load_font(&format!("{name}:regular")).or_else(|| load_font("sans")).expect("no UI font found");
        let bold = load_font(&format!("{name}:bold")).unwrap_or_else(|| regular.clone());
        TextFonts { regular, bold, glyphs: Default::default() }
    }

    pub fn width(&self, text: &str, px: f32, bold: bool) -> f32 {
        let f = if bold { &self.bold } else { &self.regular };
        text.chars().map(|c| f.advance(c, px)).sum()
    }

    pub fn rasterize(&self, text: &str, px: f32, bold: bool) -> Raster {
        let f = if bold { &self.bold } else { &self.regular };
        let lm = f.horizontal_line_metrics(px).expect("font has no horizontal metrics");
        let (asc, desc) = (lm.ascent.ceil(), (-lm.descent).ceil());
        let mut cache = self.glyphs.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > GLYPHS_KEPT {
            cache.clear();
        }
        let size = px.to_bits();
        for c in text.chars() {
            cache.entry(glyph_key(c, size, bold)).or_insert_with(|| f.rasterize(c, px));
        }
        // the advance in a glyph's metrics is the same number `width` adds up, so the line is as wide as before
        let width: f32 = text.chars().map(|c| cache[&glyph_key(c, size, bold)].0.advance_width).sum();
        let w = width.ceil() as usize + 2;
        let h = (asc + desc) as usize + 2;
        let mut bitmap = vec![0u8; w * h];
        let mut pen = 1.0f32;
        for c in text.chars() {
            let (m, bm) = &cache[&glyph_key(c, size, bold)];
            let m = *m;
            let x0 = (pen + m.xmin as f32).round() as i32;
            let y0 = asc as i32 + 1 - m.ymin - m.height as i32;
            for row in 0..m.height {
                for col in 0..m.width {
                    let (x, y) = (x0 + col as i32, y0 + row as i32);
                    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                        let d = &mut bitmap[y as usize * w + x as usize];
                        *d = d.saturating_add(bm[row * m.width + col]);
                    }
                }
            }
            pen += m.advance_width;
        }
        Raster { w, h, ascent: asc + 1.0, width: pen - 1.0, bitmap }
    }
}

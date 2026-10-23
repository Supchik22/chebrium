//! UI text for the shell: a line rasteriser on top of the glyph engine (`glyphs`). The renderer packs the results into a
//! glyph-line atlas and the glass shader draws them as quads (see `Label` in renderer.rs).

use crate::glyphs::Font;
use std::process::Command;

/// The bundled Inter faces, for programs that lay their own text out (a terminal).
pub const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.otf");
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.otf");

pub struct TextFonts {
    regular: Font,
    bold: Font,
}

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
                return TextFonts { regular, bold };
            }
        }
        let name = std::env::var("CHEBRIUM_FONT").unwrap_or_else(|_| "Inter".to_string());
        let regular = load_font(&format!("{name}:regular")).or_else(|| load_font("sans")).expect("no UI font found");
        let bold = load_font(&format!("{name}:bold")).unwrap_or_else(|| regular.clone());
        TextFonts { regular, bold }
    }

    pub fn width(&self, text: &str, px: f32, bold: bool) -> f32 {
        let f = if bold { &self.bold } else { &self.regular };
        text.chars().map(|c| f.advance(c, px)).sum()
    }

    pub fn rasterize(&self, text: &str, px: f32, bold: bool) -> Raster {
        let f = if bold { &self.bold } else { &self.regular };
        let lm = f.horizontal_line_metrics(px).expect("font has no horizontal metrics");
        let (asc, desc) = (lm.ascent.ceil(), (-lm.descent).ceil());
        let w = self.width(text, px, bold).ceil() as usize + 2;
        let h = (asc + desc) as usize + 2;
        let mut bitmap = vec![0u8; w * h];
        let mut pen = 1.0f32;
        for c in text.chars() {
            let (m, bm) = f.rasterize(c, px);
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

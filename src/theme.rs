//! The look of the desktop the program runs on, so it can follow it.
//!
//! The Memrium shell writes its current look to `~/.config/memrium/appearance.toml` (light or dark, the accent, the shape
//! of windows, the glass tint, the language) every time it changes. [`Appearance::load`] reads that file; the runtime
//! reads it again whenever it changes, so a program follows a theme switch while it is open. Without the file (the program
//! runs on another desktop) the defaults apply. `CHEBRIUM_APPEARANCE` names another file.

use crate::style::Style;
use std::path::PathBuf;
use std::time::SystemTime;

/// How the shell decorates windows: it decides what a program's own header looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Decor {
    #[default]
    Mac,
    Win11,
    Aero,
    Gnome,
    Minimal,
}

impl Decor {
    fn parse(s: &str) -> Decor {
        match s {
            "win11" => Decor::Win11,
            "aero" => Decor::Aero,
            "gnome" => Decor::Gnome,
            "minimal" => Decor::Minimal,
            _ => Decor::Mac,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub dark: bool,
    pub accent: [f32; 3],
    /// corner radius of windows and height of the header of a window
    pub radius: f32,
    pub titlebar: f32,
    pub decor: Decor,
    /// how opaque the content of a window is (the rest lets the frosted glass show through)
    pub opacity: f32,
    /// the glass of the theme: rgb tint + strength, rgb + opacity of the border
    pub glass_tint: [f32; 4],
    pub glass_border: [f32; 4],
    /// "auto" or a language code
    pub language: String,
    /// how the controls look
    pub controls: crate::style::Controls,
    /// the desktop shows a menu bar with the menus of the program in front (a program may leave its own menus to it)
    pub menubar: bool,
}

impl Default for Appearance {
    fn default() -> Appearance {
        Appearance { dark: true, accent: [0.23, 0.55, 1.0], radius: 22.0, titlebar: 40.0, decor: Decor::Mac, opacity: 0.84, glass_tint: [0.73, 0.84, 0.92, 0.11], glass_border: [0.0, 0.0, 0.0, 0.05], language: "auto".into(), controls: crate::style::Controls::default(), menubar: false }
    }
}

fn floats<const N: usize>(v: &toml::Value) -> Option<[f32; N]> {
    let a = v.as_array()?;
    if a.len() < N {
        return None;
    }
    let mut out = [0.0; N];
    for (i, x) in a.iter().take(N).enumerate() {
        out[i] = x.as_float().or_else(|| x.as_integer().map(|n| n as f64))? as f32;
    }
    Some(out)
}

impl Appearance {
    /// Where the shell leaves it.
    pub fn path() -> PathBuf {
        if let Some(p) = std::env::var_os("CHEBRIUM_APPEARANCE") {
            return p.into();
        }
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
        base.join("memrium").join("appearance.toml")
    }

    /// The file's last change, to notice a new look.
    pub fn stamp() -> Option<SystemTime> {
        std::fs::metadata(Self::path()).and_then(|m| m.modified()).ok()
    }

    /// The shell's look; without its file (another desktop is running) the defaults, with an opaque window: only the Liquid
    /// shell frosts the glass behind a translucent window, elsewhere it would just look washed out.
    pub fn load() -> Appearance {
        match std::fs::read_to_string(Self::path()) {
            Ok(t) => Self::parse(&t),
            Err(_) => Appearance { opacity: 1.0, ..Appearance::default() },
        }
    }

    /// Anything missing or malformed keeps its default.
    pub fn parse(text: &str) -> Appearance {
        let mut a = Appearance::default();
        let Ok(t) = text.parse::<toml::Table>() else { return a };
        if let Some(v) = t.get("dark").and_then(|v| v.as_bool()) {
            a.dark = v;
        }
        if let Some(v) = t.get("accent").and_then(floats::<3>) {
            a.accent = v;
        }
        let num = |k: &str| t.get(k).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64))).map(|v| v as f32);
        a.radius = num("radius").unwrap_or(a.radius);
        a.titlebar = num("titlebar").unwrap_or(a.titlebar);
        a.opacity = num("opacity").unwrap_or(a.opacity).clamp(0.0, 1.0);
        if let Some(v) = t.get("decoration").and_then(|v| v.as_str()) {
            a.decor = Decor::parse(v);
        }
        if let Some(v) = t.get("glass_tint").and_then(floats::<4>) {
            a.glass_tint = v;
        }
        if let Some(v) = t.get("glass_border").and_then(floats::<4>) {
            a.glass_border = v;
        }
        if let Some(v) = t.get("language").and_then(|v| v.as_str()) {
            a.language = v.to_string();
        }
        if let Some(v) = t.get("controls").and_then(|v| v.as_str()) {
            a.controls.kind = if v == "flat" { crate::style::ControlKind::Flat } else { crate::style::ControlKind::Keys };
        }
        a.controls.depth = num("controls_depth").unwrap_or(a.controls.depth).clamp(0.0, 1.5);
        a.controls.round = num("controls_round").unwrap_or(a.controls.round).clamp(0.0, 1.0);
        if let Some(v) = t.get("controls_accent").and_then(|v| v.as_bool()) {
            a.controls.accent_pick = v;
        }
        if let Some(v) = t.get("controls_multi").and_then(|v| v.as_bool()) {
            a.controls.multi = v;
        }
        if let Some(v) = t.get("menubar").and_then(|v| v.as_bool()) {
            a.menubar = v;
        }
        a
    }

    /// Ink and accent for the widgets.
    pub fn style(&self) -> Style {
        Style::new(self.dark, self.accent).with_controls(self.controls)
    }

    /// The colour of a window's content: the glass of the theme, frosted by the shell behind it, as dense as the
    /// theme's window opacity says.
    pub fn background(&self) -> ([f32; 3], f32) {
        // an opaque look stays opaque; a glass one is a little thinner than the shell's own windows
        let thin = if self.opacity >= 0.999 { 0.0 } else if self.dark { 0.06 } else { 0.02 };
        let alpha = if self.opacity >= 0.999 { 1.0 } else { (self.opacity - thin).max(0.30) };
        if self.dark {
            ([0.10, 0.13, 0.20], alpha)
        } else {
            ([0.97, 0.98, 1.0], alpha)
        }
    }

    /// A little lighter / darker than the background: bars and panels on top of it.
    pub fn panel(&self) -> [f32; 3] {
        if self.dark {
            [0.16, 0.19, 0.28]
        } else {
            [1.0, 1.0, 1.0]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_the_shell_writes_is_understood() {
        let a = Appearance::parse("dark = false\naccent = [0.21, 0.52, 0.89]\nradius = 12.0\ntitlebar = 46.0\ndecoration = \"gnome\"\nopacity = 0.94\nlanguage = \"uk\"\nglass_tint = [0.96, 0.96, 0.97, 0.8]\nglass_border = [0.0, 0.0, 0.0, 0.12]\n");
        assert!(!a.dark);
        assert_eq!((a.decor, a.radius, a.titlebar), (Decor::Gnome, 12.0, 46.0));
        assert_eq!(a.language, "uk");
        assert_eq!(a.glass_tint[3], 0.8);
        assert_eq!(a.accent, [0.21, 0.52, 0.89]);
    }

    #[test]
    fn missing_or_broken_input_keeps_the_defaults() {
        assert_eq!(Appearance::parse(""), Appearance::default());
        assert_eq!(Appearance::parse("this is { not toml"), Appearance::default());
        let a = Appearance::parse("dark = 3\naccent = [1.0]\nopacity = 7\n");
        assert_eq!(a.accent, Appearance::default().accent);
        assert_eq!(a.opacity, 1.0, "clamped");
    }

    #[test]
    fn a_dark_and_a_light_background_differ() {
        let dark = Appearance { dark: true, ..Default::default() };
        let light = Appearance { dark: false, ..Default::default() };
        assert!(dark.background().0[0] < light.background().0[0]);
    }
}

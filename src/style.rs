//! The few colours every widget needs, and how the controls (switches, sliders, segmented pickers) are built.

/// How the controls look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ControlKind {
    /// domed keys with a pale ring, sunk grooves: like the keys of a pocket calculator
    #[default]
    Keys,
    /// flat fills
    Flat,
}

/// What a colour is for. With a single accent they are all the accent; with several, each has a colour of its own and
/// only `Primary` follows the accent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Primary,
    /// an on / off switch, something that is fine
    Success,
    /// something that wants care
    Warning,
    /// something wrong, or today
    Danger,
    /// a second kind of thing next to the main one (an input beside an output)
    Info,
    /// a third kind
    Special,
    Pink,
    Indigo,
    Yellow,
    Grey,
}

impl Tone {
    /// The colour of this tone beside `accent` (which is `Primary`).
    pub fn colour(self, accent: [f32; 3]) -> [f32; 3] {
        match self {
            Tone::Primary => accent,
            Tone::Success => [0.20, 0.78, 0.35],
            Tone::Warning => [1.0, 0.58, 0.0],
            Tone::Danger => [1.0, 0.27, 0.23],
            Tone::Info => [0.19, 0.69, 0.78],
            Tone::Special => [0.69, 0.32, 0.87],
            Tone::Pink => [1.0, 0.24, 0.45],
            Tone::Indigo => [0.35, 0.34, 0.84],
            Tone::Yellow => [1.0, 0.80, 0.04],
            Tone::Grey => [0.56, 0.58, 0.62],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    pub kind: ControlKind,
    /// 0 .. 1: how much the keys stand out (their shadow and the dome of their face)
    pub depth: f32,
    /// 0 .. 1: how round the corners are (0 nearly square, 1 pills and discs)
    pub round: f32,
    /// the chosen segment is in the accent colour (instead of a light key)
    pub accent_pick: bool,
    /// several accent colours by context (switches green, a battery green or amber, a warning orange ...), the main one
    /// staying the accent
    pub multi: bool,
}

impl Default for Controls {
    fn default() -> Controls {
        Controls { kind: ControlKind::Keys, depth: 1.0, round: 1.0, accent_pick: false, multi: false }
    }
}

impl Controls {
    /// The colour for a tone: the accent when there is a single one.
    pub fn tone(&self, accent: [f32; 3], tone: Tone) -> [f32; 3] {
        if self.multi { tone.colour(accent) } else { accent }
    }

    /// A corner radius that is `full` at roundness 1 and a squarer one below it.
    pub fn radius(&self, full: f32) -> f32 {
        full * (0.28 + 0.72 * self.round.clamp(0.0, 1.0))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub dark: bool,
    /// text and line colour
    pub ink: [f32; 3],
    pub accent: [f32; 3],
    pub controls: Controls,
}

impl Style {
    pub fn new(dark: bool, accent: [f32; 3]) -> Style {
        let ink = if dark { [0.95, 0.96, 0.99] } else { [0.10, 0.12, 0.18] };
        Style { dark, ink, accent, controls: Controls::default() }
    }

    pub fn with_controls(mut self, controls: Controls) -> Style {
        self.controls = controls;
        self
    }
}

impl Default for Style {
    fn default() -> Style {
        Style::new(true, [0.23, 0.55, 1.0])
    }
}

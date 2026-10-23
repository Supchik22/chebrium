//! Keys and modifiers as the widgets see them (the host translates its own key events into these).

#[derive(Clone, Copy, Default, Debug)]
pub struct KeyMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Backspace,
    Delete,
    F2,
    F5,
    Home,
    End,
    PageUp,
    PageDown,
}

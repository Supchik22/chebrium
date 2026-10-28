//! A program's own window header, for programs that ask for [`Decorations::Client`](crate::app::Decorations) or
//! [`None`](crate::app::Decorations): drawn in the style the desktop uses for its windows (macOS traffic lights, GNOME
//! round buttons, Fluent flat buttons), so such a program does not look like a stranger.
//!
//! Call [`resize_borders`] first (a press on the edge starts a resize), then [`titlebar`] or [`controls`], and skip your
//! own click handling when they say the press was theirs.

use crate::app::{Edge, Host};
use crate::draw::Align;
use crate::theme::Decor;
use crate::ui::Ui;

/// Width of the border a press on starts a resize.
const BORDER: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Button {
    Close,
    Minimize,
    Maximize,
}

/// The buttons of the window in the desktop's style, laid out from the window's `x0..x1` and centred on `cy`. `alpha` fades
/// them (an overlay that hides). Returns true when a press was on one of them.
pub fn controls(ui: &mut Ui<'_>, host: &mut Host, x0: f32, x1: f32, cy: f32, alpha: f32) -> bool {
    let look = host.appearance.clone();
    // the program's own ink: white on a player's black, dark on a light window
    let ink = ui.ink;
    let mut used = false;
    // (button, rectangle) from the desktop's layout
    let buttons: Vec<(Button, [f32; 4])> = match look.decor {
        Decor::Mac => [Button::Close, Button::Minimize, Button::Maximize].iter().enumerate().map(|(i, b)| (*b, [x0 + 20.0 + i as f32 * 21.0, cy - 8.0, 16.0, 16.0])).collect(),
        Decor::Gnome => [Button::Close, Button::Maximize, Button::Minimize].iter().enumerate().map(|(i, b)| (*b, [x1 - 12.0 - 24.0 - i as f32 * 32.0, cy - 12.0, 24.0, 24.0])).collect(),
        _ => [Button::Close, Button::Maximize, Button::Minimize].iter().enumerate().map(|(i, b)| (*b, [x1 - 46.0 - i as f32 * 46.0, cy - 16.0, 46.0, 32.0])).collect(),
    };
    let any_hover = buttons.iter().any(|(_, r)| ui.hit(*r)) && alpha > 0.3;
    for (b, r) in buttons {
        // a window of one size has a maximise button that does nothing, and looks it
        let inert = b == Button::Maximize && host.fixed;
        let alpha = if inert { alpha * 0.3 } else { alpha };
        let hov = alpha > 0.3 && ui.hit(r) && !inert;
        let key = ui.id() * 64;
        let wash = ui.ease(key, if hov { 1.0 } else { 0.0 }, 22.0);
        let (mx, my) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        let icon = match b {
            Button::Close => "x",
            Button::Minimize => "minus",
            Button::Maximize => {
                if host.maximized {
                    "copy"
                } else {
                    "square"
                }
            }
        };
        match look.decor {
            Decor::Mac => {
                let c = match b {
                    Button::Close => [1.0, 0.37, 0.34],
                    Button::Minimize => [1.0, 0.75, 0.18],
                    Button::Maximize => [0.16, 0.80, 0.26],
                };
                ui.fill([mx - 6.5, my - 6.5, 13.0, 13.0], 6.5, c, alpha);
                if any_hover {
                    ui.glyph(mx, my, 9.0, icon, [0.25, 0.08, 0.06], 0.7 * alpha);
                }
            }
            Decor::Gnome => {
                ui.fill([mx - 12.0, my - 12.0, 24.0, 24.0], 12.0, ink, (0.07 + 0.09 * wash) * alpha);
                ui.glyph(mx, my, 11.0, icon, ink, 0.9 * alpha);
            }
            _ => {
                if b == Button::Close && wash > 0.01 {
                    ui.fill(r, 0.0, [0.77, 0.17, 0.17], wash * alpha);
                } else if wash > 0.01 {
                    ui.fill(r, 0.0, ink, 0.08 * wash * alpha);
                }
                ui.glyph(mx, my, 11.0, icon, if b == Button::Close && wash > 0.5 { [1.0; 3] } else { ink }, 0.9 * alpha);
            }
        }
        if ui.click && hov {
            used = true;
            match b {
                Button::Close => host.quit = true,
                Button::Minimize => host.minimize = true,
                Button::Maximize => host.toggle_maximize = !host.fixed,
            }
        }
    }
    used
}

/// A whole header across the top of the window (`w` wide): the buttons, the title, a line under it, and the drag area.
/// The height is the desktop's header height (`host.appearance.titlebar`); returns it so the content can start below.
pub fn titlebar(ui: &mut Ui<'_>, host: &mut Host, w: f32, title: &str) -> f32 {
    let look = host.appearance.clone();
    let h = look.titlebar.clamp(28.0, 56.0);
    let ink = ui.ink;
    let cy = h / 2.0;
    ui.fill([0.0, h - 1.0, w, 1.0], 0.0, ink, 0.08);
    let on_button = controls(ui, host, 0.0, w, cy, 1.0);
    let (tx, align) = match look.decor {
        Decor::Win11 | Decor::Aero => (16.0, Align::Left),
        _ => (w / 2.0, Align::Center),
    };
    let max_w = (w - 200.0).max(60.0);
    let mut t = title.to_string();
    while t.chars().count() > 1 && ui.measure_raw(&t, 13.0, true) > max_w {
        t.pop();
    }
    ui.text_raw(tx, cy + 4.5, &t, 13.0, look.decor != Decor::Win11, ink, 0.9, align);
    // a press on the header (not on a button) moves the window; twice maximises it
    if ui.click && !on_button && ui.mouse[1] >= 0.0 && ui.mouse[1] < h && ui.mouse[0] >= 0.0 && ui.mouse[0] < w {
        if ui.double {
            host.toggle_maximize = !host.fixed;
        } else {
            host.drag = true;
        }
    }
    h
}

/// For a program whose content runs up under the title row (a sidebar down the full height, a toolbar in the header): the
/// window's buttons are drawn over it in the desktop's style, and a press on the bare part of the top `bar_h` px moves
/// the window (twice maximises it). Call it last in the frame, after the widgets; `used` says one of them took the press.
/// On a Mac-style desktop the buttons sit at the left, so keep the first 80 px of the sidebar's top clear.
pub fn integrated(ui: &mut Ui<'_>, host: &mut Host, size: [f32; 2], bar_h: f32, used: bool) -> bool {
    let [w, _] = size;
    let on_button = controls(ui, host, 0.0, w, bar_h / 2.0, 1.0);
    let bare = ui.click && !used && !on_button && ui.mouse[1] >= BORDER && ui.mouse[1] < bar_h && ui.mouse[0] >= BORDER && ui.mouse[0] < w - BORDER;
    if bare {
        if ui.double {
            host.toggle_maximize = !host.fixed;
        } else {
            host.drag = true;
        }
    }
    on_button || bare
}

/// A press on the edge or corner of the window starts a resize. Returns true when it did (the press is used up).
pub fn resize_borders(ui: &Ui<'_>, host: &mut Host, size: [f32; 2]) -> bool {
    if !ui.click || host.maximized || host.fixed {
        return false;
    }
    let [x, y] = ui.mouse;
    let [w, h] = size;
    let (l, r, t, b) = (x < BORDER, x > w - BORDER, y < BORDER, y > h - BORDER);
    let edge = match (l, r, t, b) {
        (true, _, true, _) => Edge::TopLeft,
        (_, true, true, _) => Edge::TopRight,
        (true, _, _, true) => Edge::BottomLeft,
        (_, true, _, true) => Edge::BottomRight,
        (true, ..) => Edge::Left,
        (_, true, ..) => Edge::Right,
        (_, _, true, _) => Edge::Top,
        (_, _, _, true) => Edge::Bottom,
        _ => return false,
    };
    host.resize = Some(edge);
    true
}

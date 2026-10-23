//! Parts for programs that look like a little device: seven-segment digits for an LCD, and keys with a pale ring and a
//! slightly domed face (the calculator, the snake game).

use crate::ui::Ui;

/// Segments of a seven-segment digit, as bits: a top, b top right, c bottom right, d bottom, e bottom left, f top left,
/// g middle.
pub fn segments(c: char) -> u8 {
    const A: u8 = 1;
    const B: u8 = 2;
    const C: u8 = 4;
    const D: u8 = 8;
    const E: u8 = 16;
    const F: u8 = 32;
    const G: u8 = 64;
    match c {
        '0' => A | B | C | D | E | F,
        '1' => B | C,
        '2' => A | B | D | E | G,
        '3' => A | B | C | D | G,
        '4' => B | C | F | G,
        '5' => A | C | D | F | G,
        '6' => A | C | D | E | F | G,
        '7' => A | B | C,
        '8' => A | B | C | D | E | F | G,
        '9' => A | B | C | D | F | G,
        '-' => G,
        'E' => A | D | E | F | G,
        'r' => E | G,
        'o' => C | D | E | G,
        _ => 0,
    }
}

/// One seven-segment digit with its top left corner at (x, y), `h` tall; lit segments dark, the others a faint ghost.
pub fn digit_cell(ui: &mut Ui<'_>, x: f32, y: f32, w: f32, h: f32, c: char, ink: [f32; 3]) {
    let t = h * 0.12;
    let on = segments(c);
    let seg = [
        [x + t * 0.9, y, w - t * 1.8, t],
        [x + w - t, y + t * 0.7, t, h / 2.0 - t * 0.95],
        [x + w - t, y + h / 2.0 + t * 0.25, t, h / 2.0 - t * 0.95],
        [x + t * 0.9, y + h - t, w - t * 1.8, t],
        [x, y + h / 2.0 + t * 0.25, t, h / 2.0 - t * 0.95],
        [x, y + t * 0.7, t, h / 2.0 - t * 0.95],
        [x + t * 0.9, y + h / 2.0 - t / 2.0, w - t * 1.8, t],
    ];
    for (i, r) in seg.iter().enumerate() {
        let lit = on & (1 << i) != 0;
        ui.fill(*r, t / 2.0, ink, if lit { 0.92 } else { 0.055 });
    }
}

/// A key's body: a soft shadow, a pale ring and a slightly domed face, like the keys of the calculator.
pub fn key_body(ui: &mut Ui<'_>, r: [f32; 4], face: [f32; 3], wash: f32, flash: f32) {
    key_body_with(ui, r, face, [0.99, 1.0, 1.0], ([0.30, 0.38, 0.46], 0.22), wash, flash);
}

/// The same with its own colour for the ring and the shadow (colour, strength): keys that suit a dark window, or
/// that are pressed in and throw less shadow.
pub fn key_body_with(ui: &mut Ui<'_>, r: [f32; 4], face: [f32; 3], ring: [f32; 3], shadow: ([f32; 3], f32), wash: f32, flash: f32) {
    key_body_shaped(ui, r, face, ring, shadow, wash, flash, 1.0, 1.0);
}

/// A key with its roundness (0 nearly square, 1 as round as it can be) and depth (0 flat, 1 as domed as the calculator's).
#[allow(clippy::too_many_arguments)]
pub fn key_body_shaped(ui: &mut Ui<'_>, r: [f32; 4], face: [f32; 3], ring: [f32; 3], shadow: ([f32; 3], f32), wash: f32, flash: f32, round: f32, depth: f32) {
    let full = r[3].min(r[2]) / 2.0;
    let rad = full * (0.28 + 0.72 * round.clamp(0.0, 1.0));
    let depth = depth.clamp(0.0, 1.5);
    ui.fill([r[0], r[1] + 3.0 * depth.min(1.0), r[2], r[3]], rad, shadow.0, shadow.1 * depth.min(1.2));
    ui.fill(r, rad, ring, 1.0);
    let inner = [r[0] + 2.5, r[1] + 2.5, r[2] - 5.0, r[3] - 5.0];
    let irad = (inner[3].min(inner[2]) / 2.0) * (0.28 + 0.72 * round.clamp(0.0, 1.0));
    let lift = 0.025 * wash - 0.05 * flash;
    let (up, down) = (0.05 * depth, 0.02 * depth);
    let (top, bot) = ([face[0] + up + lift, face[1] + up + lift, face[2] + up + lift], [face[0] - down + lift, face[1] - down + lift, face[2] - down + lift]);
    ui.grad(inner, irad, top.map(|v| v.clamp(0.0, 1.0)), bot.map(|v| v.clamp(0.0, 1.0)), 1.0);
}

/// A key for a dark body, like a soft rubber key lit from above: a rim that is bright along its top and falls away
/// below, a slightly domed face with a faint sheen, and a deep shadow. `face` is the colour of the face's middle.
#[allow(clippy::too_many_arguments)]
pub fn key_body_dark(ui: &mut Ui<'_>, r: [f32; 4], face: [f32; 3], wash: f32, flash: f32, round: f32, depth: f32) {
    let full = r[3].min(r[2]) / 2.0;
    let k = 0.28 + 0.72 * round.clamp(0.0, 1.0);
    let rad = full * k;
    let depth = depth.clamp(0.0, 1.5);
    let lift = 0.03 * wash - 0.05 * flash;
    let tone = |d: f32| [(face[0] + d + lift).clamp(0.0, 1.0), (face[1] + d + lift).clamp(0.0, 1.0), (face[2] + d + lift).clamp(0.0, 1.0)];
    ui.fill([r[0], r[1] + 3.5 * depth.min(1.0), r[2], r[3]], rad, [0.0; 3], 0.55 * depth.min(1.2));
    // the rim: lit along the top, dark along the bottom
    ui.grad(r, rad, tone(0.20 * depth.max(0.3)), tone(-0.12), 1.0);
    let inner = [r[0] + 2.0, r[1] + 2.0, r[2] - 4.0, r[3] - 4.5];
    let irad = (inner[3].min(inner[2]) / 2.0) * k;
    ui.grad(inner, irad, tone(0.045 * depth), tone(-0.035 * depth), 1.0);
    // a faint sheen on the upper part of the dome
    ui.fill([inner[0] + inner[2] * 0.18, inner[1] + 1.5, inner[2] * 0.64, inner[3] * 0.30], irad * 0.7, [1.0; 3], 0.045 * depth.min(1.2));
}

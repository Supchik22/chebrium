//! Keyed easing for things that glide: a sliding highlight, a switch knob, a hover wash.
//!
//! Each animated value is addressed by a `u64` key (the widgets derive theirs from their draw order). A value
//! eases towards its target with an exponential curve, snaps once it is close enough and reports whether
//! anything is still moving, so the host knows when it may stop redrawing.

use std::collections::HashMap;

#[derive(Default)]
pub struct Anims {
    values: HashMap<u64, f32>,
    moving: bool,
}

impl Anims {
    /// Glide the value under `key` towards `target`; `speed` is how fast, per second (15 is brisk). A value seen
    /// for the first time starts at its target.
    pub fn ease(&mut self, key: u64, target: f32, speed: f32, dt: f32) -> f32 {
        let v = self.values.entry(key).or_insert(target);
        *v += (target - *v) * (1.0 - (-speed * dt).exp());
        if (*v - target).abs() < 0.002 {
            *v = target;
        } else {
            self.moving = true;
        }
        *v
    }

    /// Put the value under `key` at `value` at once, with no glide (a new picture that should not slide in).
    pub fn set(&mut self, key: u64, value: f32) {
        self.values.insert(key, value);
    }

    /// Forget everything (a page changed: its controls appear where they belong at once).
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Did a value move since the last call? Resets the flag.
    pub fn take_moving(&mut self) -> bool {
        std::mem::take(&mut self.moving)
    }
}

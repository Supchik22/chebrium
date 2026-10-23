//! Damped spring used for every "liquid" motion: hover growth, dock magnification,
//! bounce and jelly wobble. Underdamped on purpose so things overshoot a little.

#[derive(Clone, Copy)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
    pub target: f32,
    k: f32,
    c: f32,
}

impl Spring {
    /// `stiffness` ~ 150..400, `damping_ratio` < 1 overshoots (0.55 is nicely jelly).
    pub fn new(x: f32, stiffness: f32, damping_ratio: f32) -> Self {
        Self { x, v: 0.0, target: x, k: stiffness, c: 2.0 * damping_ratio * stiffness.sqrt() }
    }

    /// Jump to the target at once (an animation that is switched off).
    pub fn snap(&mut self) {
        self.x = self.target;
        self.v = 0.0;
    }

    pub fn kick(&mut self, impulse: f32) {
        self.v += impulse;
    }

    /// Close enough to the target and nearly still: further motion would be invisible.
    const REST: f32 = 5e-4;

    pub fn at_rest(&self) -> bool {
        self.v == 0.0 && self.x == self.target
    }

    pub fn update(&mut self, dt: f32) {
        if self.at_rest() {
            return;
        }
        let mut t = dt.min(0.1);
        while t > 0.0 {
            let h = t.min(1.0 / 240.0);
            let a = -self.k * (self.x - self.target) - self.c * self.v;
            self.v += a * h;
            self.x += self.v * h;
            t -= h;
        }
        // an underdamped spring would otherwise ring at ever smaller amplitudes forever, which keeps every frame
        // "different" (and the GPU busy) for no visible reason
        if (self.x - self.target).abs() < Self::REST && self.v.abs() < Self::REST * 4.0 {
            self.x = self.target;
            self.v = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_underdamped_spring_comes_to_rest_exactly() {
        // otherwise the picture would differ from the last one forever and the shell could never go idle
        let mut s = Spring::new(0.0, 190.0, 0.32);
        s.target = 1.0;
        for _ in 0..600 {
            s.update(1.0 / 60.0);
        }
        assert!(s.at_rest());
        assert_eq!(s.x, 1.0);
    }

    #[test]
    fn a_kick_moves_a_resting_spring() {
        let mut s = Spring::new(2.0, 300.0, 0.5);
        assert!(s.at_rest());
        s.kick(10.0);
        s.update(0.016);
        assert!(s.x != 2.0);
    }
}

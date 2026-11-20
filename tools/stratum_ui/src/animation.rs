//! Animation primitives.
//!
//! [`Tween`] is a fixed-duration eased transition, driven from the frame loop
//! by advancing with a delta time. Values are plain `f32`, so it animates
//! anything: position, size, opacity, colour components, or shading
//! parameters.

/// Easing curve for a [`Tween`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    EaseIn,
    EaseOut,
    /// Slight overshoot then settle, like QML's `OutBack`.
    OutBack,
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::OutBack => {
                const C1: f32 = 1.70158;
                const C3: f32 = C1 + 1.0;
                let p = t - 1.0;
                1.0 + C3 * p * p * p + C1 * p * p
            }
        }
    }
}

/// A fixed-duration eased transition.
#[derive(Clone, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    duration: f32,
    elapsed: f32,
    easing: Easing,
    value: f32,
}

impl Default for Tween {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Tween {
    pub fn new(initial: f32) -> Self {
        Self {
            from: initial,
            to: initial,
            duration: 0.0,
            elapsed: 0.0,
            easing: Easing::EaseOut,
            value: initial,
        }
    }

    /// Start moving toward `target`.
    pub fn to(&mut self, target: f32, duration_seconds: f32, easing: Easing) {
        if (target - self.to).abs() < f32::EPSILON && self.duration > 0.0 {
            return;
        }
        self.from = self.value;
        self.to = target;
        self.duration = duration_seconds.max(0.0);
        self.elapsed = 0.0;
        self.easing = easing;
        if self.duration == 0.0 {
            self.value = target;
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    /// Jump to `value`, cancelling any motion.
    pub fn snap(&mut self, value: f32) {
        self.from = value;
        self.to = value;
        self.elapsed = 0.0;
        self.duration = 0.0;
        self.value = value;
    }

    /// Advance by `dt` seconds. Returns true while still animating.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.duration <= 0.0 || self.elapsed >= self.duration {
            self.value = self.to;
            return false;
        }
        self.elapsed += dt;
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0);
        self.value = self.from + (self.to - self.from) * self.easing.apply(t);
        if t >= 1.0 {
            self.value = self.to;
            false
        } else {
            true
        }
    }

    pub fn is_animating(&self) -> bool {
        self.duration > 0.0 && self.elapsed < self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tween_reaches_target_and_stops() {
        let mut tween = Tween::new(0.0);
        tween.to(10.0, 0.1, Easing::EaseOut);
        for _ in 0..20 {
            tween.tick(0.01);
        }
        assert!((tween.value() - 10.0).abs() < 1e-3);
        assert!(!tween.is_animating());
    }

    #[test]
    fn zero_duration_is_immediate() {
        let mut tween = Tween::new(0.0);
        tween.to(5.0, 0.0, Easing::OutBack);
        assert_eq!(tween.value(), 5.0);
        assert!(!tween.tick(0.016));
    }

    #[test]
    fn easing_curves_are_bounded_at_ends() {
        for easing in [Easing::EaseIn, Easing::EaseOut, Easing::OutBack] {
            assert!((easing.apply(0.0) - 0.0).abs() < 1e-5);
            assert!((easing.apply(1.0) - 1.0).abs() < 1e-5);
        }
    }
}

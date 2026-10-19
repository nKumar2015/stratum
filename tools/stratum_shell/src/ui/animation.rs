//! Animation primitives.
//!
//! Two kinds, matching what the QML shell had:
//!
//! - [`Tween`] — fixed-duration easing (the QML `NumberAnimation`/`ColorAnimation`).
//! - [`Spring`] — a damped spring that settles naturally (loaf-fm's
//!   `SpringAnimation`), which is what makes UI motion feel physical rather than
//!   mechanical.
//!
//! Both are driven from the frame loop by advancing with a delta time. Values
//! are plain `f32`, so they animate anything: position, size, opacity, colour
//! components, or shading parameters.

/// Easing curve for a [`Tween`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Slight overshoot then settle, like QML's `OutBack`.
    OutBack,
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - 2.0 * (1.0 - t) * (1.0 - t)
                }
            }
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
pub struct Tween {    from: f32,
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

    pub fn target(&self) -> f32 {
        self.to
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

/// A damped spring, following the same parameterisation as Qt's
/// `SpringAnimation` (stiffness and damping ratio) so behaviour is familiar.
#[derive(Clone, Debug)]
pub struct Spring {
    value: f32,
    target: f32,
    velocity: f32,
    /// Spring constant. Higher is snappier.
    stiffness: f32,
    /// Damping ratio: 1.0 is critically damped, below 1 overshoots.
    damping: f32,
}

impl Spring {
    pub fn new(initial: f32) -> Self {
        Self {
            value: initial,
            target: initial,
            velocity: 0.0,
            stiffness: 4.0,
            damping: 0.7,
        }
    }

    pub fn with_params(mut self, stiffness: f32, damping: f32) -> Self {
        self.stiffness = stiffness.max(0.01);
        self.damping = damping.max(0.0);
        self
    }

    pub fn to(&mut self, target: f32) {
        self.target = target;
    }

    /// Jump immediately, cancelling any motion.
    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Advance by `dt` seconds, semi-implicit Euler. Returns true while moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        // Clamp the step so a long pause does not explode the integration.
        let dt = dt.min(0.05);
        let displacement = self.value - self.target;

        // a = -k*x - c*v, with c = 2 * damping * sqrt(k) for the ratio form.
        let c = 2.0 * self.damping * self.stiffness.sqrt();
        let accel = -self.stiffness * displacement - c * self.velocity;
        self.velocity += accel * dt;
        self.value += self.velocity * dt;

        // Settled when close and slow.
        let settled = displacement.abs() < 0.001 && self.velocity.abs() < 0.001;
        if settled {
            self.value = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }
}

/// A spring whose rest position is a colour, for hover and state fades.
#[derive(Clone, Debug)]
pub struct ColorSpring {
    channels: [Spring; 4],
}

impl ColorSpring {
    pub fn new(color: crate::render::display_list::Color) -> Self {
        Self {
            channels: [
                Spring::new(color.r),
                Spring::new(color.g),
                Spring::new(color.b),
                Spring::new(color.a),
            ],
        }
    }

    pub fn to(&mut self, color: crate::render::display_list::Color) {
        self.channels[0].to(color.r);
        self.channels[1].to(color.g);
        self.channels[2].to(color.b);
        self.channels[3].to(color.a);
    }

    pub fn value(&self) -> crate::render::display_list::Color {
        crate::render::display_list::Color::rgba(
            self.channels[0].value().clamp(0.0, 1.0),
            self.channels[1].value().clamp(0.0, 1.0),
            self.channels[2].value().clamp(0.0, 1.0),
            self.channels[3].value().clamp(0.0, 1.0),
        )
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for channel in &mut self.channels {
            moving |= channel.tick(dt);
        }
        moving
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tween_reaches_target_and_stops() {
        let mut tween = Tween::new(0.0);
        tween.to(10.0, 0.1, Easing::Linear);
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
        for easing in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::OutBack,
        ] {
            assert!((easing.apply(0.0) - 0.0).abs() < 1e-5);
            assert!((easing.apply(1.0) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn spring_settles_at_target() {
        let mut spring = Spring::new(0.0);
        spring.to(1.0);
        for _ in 0..600 {
            if !spring.tick(0.016) {
                break;
            }
        }
        assert!((spring.value() - 1.0).abs() < 1e-2, "value {}", spring.value());
    }

    #[test]
    fn underdamped_spring_overshoots() {
        let mut spring = Spring::new(0.0).with_params(20.0, 0.3);
        spring.to(1.0);
        let mut peak: f32 = 0.0;
        for _ in 0..300 {
            spring.tick(0.016);
            peak = peak.max(spring.value());
        }
        assert!(peak > 1.02, "expected overshoot, peak was {peak}");
    }

    #[test]
    fn color_spring_converges() {
        use crate::render::display_list::Color;
        let mut spring = ColorSpring::new(Color::BLACK);
        spring.to(Color::WHITE);
        for _ in 0..600 {
            spring.tick(0.016);
        }
        assert!((spring.value().r - 1.0).abs() < 0.02);
    }
}

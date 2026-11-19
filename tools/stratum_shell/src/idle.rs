//! What being idle does: dim the screens, lock, turn the screens off, and
//! suspend, each after its own timeout in `[idle]`, and how input undoes it.
//!
//! Pure and tested. The compositor tells the shell when each stage's timeout
//! passes with no input (`ext-idle-notify-v1`, one notification per stage)
//! and when input comes back; `wayland/state/idle.rs` feeds those edges in
//! here and carries out the [`Effect`]s. Clients that inhibit idling (a video
//! player) hold every stage off, because the compositor then never reports
//! idle at all.

use std::time::Duration;

use crate::config::IdleConfig;

/// One idle stage, each with its own timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Dim,
    Lock,
    Dpms,
    Suspend,
}

/// What a stage starting or ending asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    DimOn,
    DimOff,
    Lock,
    DpmsOff,
    DpmsOn,
    Suspend,
}

/// The stages `config` enables and their timeouts in milliseconds, for the
/// compositor's idle notifications. A timeout of 0 turns a stage off.
pub fn timeouts(config: &IdleConfig) -> Vec<(Stage, u32)> {
    [
        (Stage::Dim, config.dim_s),
        (Stage::Lock, config.lock_s),
        (Stage::Dpms, config.dpms_s),
        (Stage::Suspend, config.suspend_s),
    ]
    .into_iter()
    .filter(|&(_, seconds)| seconds > 0)
    .map(|(stage, seconds)| (stage, seconds.saturating_mul(1000)))
    .collect()
}

/// Which stages have taken effect, and whether the user has asked to stay
/// awake.
#[derive(Debug, Default)]
pub struct Idle {
    inhibited: bool,
    dimmed: bool,
    screens_off: bool,
}

impl Idle {
    pub fn inhibited(&self) -> bool {
        self.inhibited
    }

    /// `stage`'s timeout passed with no input. `on_battery` and
    /// `suspend_on_ac` decide whether the suspend stage acts.
    pub fn idled(&mut self, stage: Stage, on_battery: bool, suspend_on_ac: bool) -> Option<Effect> {
        if self.inhibited {
            return None;
        }
        match stage {
            Stage::Dim if !self.dimmed => {
                self.dimmed = true;
                Some(Effect::DimOn)
            }
            Stage::Dpms if !self.screens_off => {
                self.screens_off = true;
                Some(Effect::DpmsOff)
            }
            Stage::Lock => Some(Effect::Lock),
            Stage::Suspend if on_battery || suspend_on_ac => Some(Effect::Suspend),
            _ => None,
        }
    }

    /// Input came back after `stage` had idled. The lock stays until the
    /// password, and a suspend has already happened.
    pub fn resumed(&mut self, stage: Stage) -> Option<Effect> {
        match stage {
            Stage::Dim if self.dimmed => {
                self.dimmed = false;
                Some(Effect::DimOff)
            }
            Stage::Dpms if self.screens_off => {
                self.screens_off = false;
                Some(Effect::DpmsOn)
            }
            _ => None,
        }
    }

    /// Stay awake, or stop. Staying awake undoes a dim or a blank screen
    /// already in effect; stopping waits for the next idle period.
    pub fn set_inhibited(&mut self, inhibited: bool) -> Vec<Effect> {
        self.inhibited = inhibited;
        if inhibited {
            self.reset()
        } else {
            Vec::new()
        }
    }

    /// Undo whatever is in effect, for new timeouts or a shell without idle.
    pub fn reset(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if std::mem::take(&mut self.dimmed) {
            effects.push(Effect::DimOff);
        }
        if std::mem::take(&mut self.screens_off) {
            effects.push(Effect::DpmsOn);
        }
        effects
    }
}

/// The dim layer's opacity `elapsed` into a fade of `fade` towards `opacity`.
/// It eases out, so the change is noticeable at once and the user can move
/// before the screen gets dark.
pub fn dim_alpha(elapsed: Duration, fade: Duration, opacity: f32) -> f32 {
    let t = if fade.is_zero() {
        1.0
    } else {
        elapsed.as_secs_f32() / fade.as_secs_f32()
    };
    crate::ui::animation::Easing::EaseOut.apply(t) * opacity.clamp(0.0, 1.0)
}

/// Opacity steps the dim layer is drawn in. Each step is a 1×1 buffer, so a
/// fade commits at most this many times, not once per frame.
pub const DIM_LEVELS: u8 = 64;

/// `alpha` as one of [`DIM_LEVELS`] + 1 steps, 0 being clear.
pub fn dim_level(alpha: f32) -> u8 {
    (alpha.clamp(0.0, 1.0) * f32::from(DIM_LEVELS)).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_take_effect_once_and_input_undoes_dim_and_dpms() {
        let mut idle = Idle::default();
        assert_eq!(idle.idled(Stage::Dim, true, false), Some(Effect::DimOn));
        assert_eq!(idle.idled(Stage::Dim, true, false), None, "already dimmed");
        assert_eq!(idle.idled(Stage::Lock, true, false), Some(Effect::Lock));
        assert_eq!(idle.idled(Stage::Dpms, true, false), Some(Effect::DpmsOff));
        assert_eq!(idle.resumed(Stage::Dim), Some(Effect::DimOff));
        assert_eq!(idle.resumed(Stage::Lock), None, "only the password unlocks");
        assert_eq!(idle.resumed(Stage::Dpms), Some(Effect::DpmsOn));
        assert_eq!(idle.resumed(Stage::Dpms), None, "screens already on");
    }

    #[test]
    fn suspend_follows_the_power_source() {
        let mut idle = Idle::default();
        assert_eq!(idle.idled(Stage::Suspend, false, false), None, "on AC");
        assert_eq!(idle.idled(Stage::Suspend, false, true), Some(Effect::Suspend));
        assert_eq!(idle.idled(Stage::Suspend, true, false), Some(Effect::Suspend));
    }

    #[test]
    fn staying_awake_blocks_every_stage_and_undoes_dim_and_dpms() {
        let mut idle = Idle::default();
        idle.idled(Stage::Dim, true, false);
        idle.idled(Stage::Dpms, true, false);
        assert_eq!(idle.set_inhibited(true), vec![Effect::DimOff, Effect::DpmsOn]);
        for stage in [Stage::Dim, Stage::Lock, Stage::Dpms, Stage::Suspend] {
            assert_eq!(idle.idled(stage, true, true), None, "{stage:?}");
        }
        // The resumes that follow have nothing left to undo.
        assert_eq!(idle.resumed(Stage::Dim), None);
        assert!(idle.set_inhibited(false).is_empty());
        assert_eq!(idle.idled(Stage::Lock, true, false), Some(Effect::Lock));
    }

    #[test]
    fn zero_turns_a_stage_off() {
        let mut config = crate::config::Config::test_defaults().idle;
        config.dim_s = 0;
        config.lock_s = 300;
        config.dpms_s = 0;
        config.suspend_s = 900;
        assert_eq!(timeouts(&config), vec![(Stage::Lock, 300_000), (Stage::Suspend, 900_000)]);
    }

    #[test]
    fn the_dim_fades_in_to_its_opacity() {
        let fade = Duration::from_secs(2);
        assert_eq!(dim_alpha(Duration::ZERO, fade, 0.6), 0.0);
        let half = dim_alpha(Duration::from_secs(1), fade, 0.6);
        assert!(half > 0.3 && half < 0.6, "eases out: {half}");
        assert_eq!(dim_alpha(Duration::from_secs(5), fade, 0.6), 0.6);
        assert_eq!(dim_alpha(Duration::ZERO, Duration::ZERO, 0.6), 0.6, "no fade");
        assert_eq!(dim_level(0.0), 0);
        assert_eq!(dim_level(1.0), DIM_LEVELS);
    }
}

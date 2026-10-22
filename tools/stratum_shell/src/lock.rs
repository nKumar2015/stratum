//! The lock screen.
//!
//! Locking goes through `ext-session-lock-v1`: the compositor hides every
//! window and shows only our lock surfaces until we unlock, and keeps the
//! session locked if the shell dies. This module holds what the surfaces show
//! and how typing behaves; `wayland::state` owns the protocol objects and runs
//! the PAM check on a worker thread.
//!
//! Matching `modules/LockScreen.qml`: a blurred, dimmed wallpaper, the time,
//! a round avatar, a password field, and "Incorrect password, try again." on
//! failure. One input is shared by every output, so typing works on whichever
//! monitor the compositor gives keyboard focus.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::pam::Secret;
use crate::render::display_list::{Color, DisplayList, Image, Rect};
use crate::ui::password_field::{FieldKey, FieldStyle, PasswordField};
use crate::ui::{self, PaintCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Unlocked,
    /// Lock requested; waiting for the compositor to confirm.
    Locking,
    Locked,
}

/// What a keypress asks the caller to do.
#[derive(Debug)]
pub enum KeyOutcome {
    /// Nothing changed.
    Ignored,
    /// The field changed and the surfaces need repainting.
    Changed,
    /// Check this password.
    Submit(Secret),
}

/// Compared by variant only: passwords are never compared outside PAM.
impl PartialEq for KeyOutcome {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

pub struct LockScreen {
    phase: Phase,
    field: PasswordField,
    /// A PAM check is running; input is held until it answers.
    checking: bool,
    /// The last attempt failed. Cleared by the next keypress.
    failed: bool,
    /// PAM's own explanation for a failure, when it gave one that is more
    /// specific than a wrong password (an expired or locked account).
    reason: Option<String>,
    /// The minute last painted, so the clock repaints only when it changes.
    painted_minute: Option<i64>,
    metrics: crate::ui::style::LockMetrics,
}

impl LockScreen {
    pub fn new() -> Self {
        Self {
            phase: Phase::Unlocked,
            field: PasswordField::new(),
            checking: false,
            failed: false,
            reason: None,
            painted_minute: None,
            metrics: crate::ui::style::LockMetrics::default(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &crate::ui::style::LockMetrics) {
        self.metrics = metrics.clone();
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// True from the lock request until the unlock, i.e. whenever keys and
    /// clicks belong to the lock screen.
    pub fn active(&self) -> bool {
        self.phase != Phase::Unlocked
    }

    /// Start locking. Returns false when already locking or locked, so a
    /// repeated keybind does not create a second lock.
    pub fn request(&mut self) -> bool {
        if self.active() {
            return false;
        }
        self.phase = Phase::Locking;
        self.reset_input();
        true
    }

    /// The compositor confirmed the lock.
    pub fn confirm(&mut self) {
        if self.phase == Phase::Locking {
            self.phase = Phase::Locked;
        }
    }

    /// The lock ended: unlocked, or refused by the compositor.
    pub fn finish(&mut self) {
        self.phase = Phase::Unlocked;
        self.reset_input();
    }

    fn reset_input(&mut self) {
        self.field.clear();
        self.checking = false;
        self.failed = false;
        self.reason = None;
        self.painted_minute = None;
    }

    /// Handle a keypress. `text` is what the key types under the current
    /// layout and modifiers.
    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
    ) -> KeyOutcome {
        // Typing before the compositor confirms would be lost on a surface
        // that is not shown yet; typing during a check would race it.
        if self.phase != Phase::Locked || self.checking {
            return KeyOutcome::Ignored;
        }
        match self.field.key(keysym, text, ctrl) {
            FieldKey::Ignored => KeyOutcome::Ignored,
            FieldKey::Changed => {
                self.failed = false;
                KeyOutcome::Changed
            }
            FieldKey::Submit(password) => {
                self.checking = true;
                self.failed = false;
                self.reason = None;
                KeyOutcome::Submit(password)
            }
            // There is nothing to close on a lock screen; Escape clears.
            FieldKey::Escape => {
                self.field.clear();
                self.failed = false;
                KeyOutcome::Changed
            }
        }
    }

    /// The PAM check answered. Returns true when the session may unlock.
    pub fn auth_result(&mut self, result: Result<(), crate::pam::AuthError>) -> bool {
        self.checking = false;
        match result {
            Ok(()) => true,
            Err(err) => {
                self.failed = true;
                // A plain wrong password gets the QML wording; anything else
                // (an expired account, a broken module) is worth showing.
                self.reason = (!err.wrong_password).then_some(err.message);
                false
            }
        }
    }

    /// Whether the clock needs repainting for `minute` (minutes since epoch).
    pub fn clock_changed(&mut self, minute: i64) -> bool {
        if self.painted_minute == Some(minute) {
            return false;
        }
        self.painted_minute = Some(minute);
        true
    }

    /// The status line under the field.
    fn status(&self) -> Option<String> {
        if self.checking {
            return Some("Checking…".to_string());
        }
        if self.failed {
            return Some(
                self.reason
                    .clone()
                    .unwrap_or_else(|| "Incorrect password, try again.".to_string()),
            );
        }
        None
    }

    /// Vertical layout of the centred column, shared by painting and tests.
    fn layout(&self, surface: Rect) -> Layout {
        let m = &self.metrics;
        let clock_h = m.clock * 1.25;
        let label_h = m.label * 1.4;
        let status_h = m.status * 1.4;
        let total = clock_h
            + m.spacing
            + m.avatar
            + m.spacing
            + label_h
            + m.spacing
            + m.field_height
            + m.spacing
            + status_h;
        let x = surface.x + surface.width * 0.5;
        let mut y = surface.y + (surface.height - total) * 0.5;
        let row = |y: f32, height: f32, width: f32| Rect {
            x: x - width * 0.5,
            y,
            width,
            height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        let clock = row(y, clock_h, surface.width);
        y += clock_h + m.spacing;
        let avatar = Rect {
            radius: m.avatar * 0.5,
            ..row(y, m.avatar, m.avatar)
        };
        y += m.avatar + m.spacing;
        let label = row(y, label_h, surface.width);
        y += label_h + m.spacing;
        let field = Rect {
            radius: m.field_radius,
            ..row(y, m.field_height, m.field_width)
        };
        y += m.field_height + m.spacing;
        let status = row(y, status_h, surface.width);
        Layout {
            clock,
            avatar,
            label,
            field,
            status,
        }
    }

    pub fn paint(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        art: &Artwork,
        time: &str,
    ) {
        let m = &self.metrics;
        let theme = ctx.theme;

        // The backdrop is opaque even without a wallpaper: a lock surface
        // that showed what is underneath would defeat the point.
        list.push_rect(Rect {
            color: theme.bg_dark.with_alpha(1.0),
            ..surface
        });
        if let Some(backdrop) = &art.backdrop {
            list.push_image(cover(backdrop, surface));
        }
        list.push_rect(Rect {
            color: theme.bg_dark.with_alpha(m.scrim_alpha),
            ..surface
        });

        let layout = self.layout(surface);
        ui::paint_text_centered_weighted(
            ctx,
            list,
            time,
            layout.clock,
            m.clock,
            ctx.metrics.typography.weight_bold,
            theme.primary,
        );

        // Avatar: a ring in the primary colour around the picture, or around
        // a user glyph when there is no picture.
        list.push_rect(Rect {
            color: theme.primary,
            ..layout.avatar
        });
        let inner = inset(layout.avatar, m.avatar_border);
        list.push_rect(Rect {
            color: theme.bg_widget,
            ..inner
        });
        match &art.avatar {
            Some(avatar) => list.push_image(Image {
                x: inner.x,
                y: inner.y,
                width_logical: inner.width,
                height_logical: inner.height,
                ..avatar.clone()
            }),
            None => ui::paint_text_centered(
                ctx,
                list,
                "\u{f007}",
                inner,
                m.avatar * 0.4,
                theme.text_muted,
            ),
        }

        ui::paint_text_centered_weighted(
            ctx,
            list,
            "Enter Password",
            layout.label,
            m.label,
            ctx.metrics.typography.weight_bold,
            theme.secondary,
        );

        let border = if self.failed {
            theme.error
        } else if self.checking {
            theme.border_inactive
        } else {
            theme.border_active
        };
        list.push_rect(Rect {
            color: border,
            ..layout.field
        });
        list.push_rect(Rect {
            color: theme.bg_widget,
            ..inset(layout.field, m.field_border)
        });
        self.field.paint(
            ctx,
            list,
            layout.field,
            FieldStyle {
                text: m.field_text,
                padding: m.field_padding,
                caret_width: m.caret_width,
            },
            self.checking,
        );

        if let Some(status) = self.status() {
            ui::paint_text_centered(
                ctx,
                list,
                &status,
                layout.status,
                m.status,
                if self.failed {
                    theme.error
                } else {
                    theme.text_muted
                },
            );
        }
    }
}

impl Default for LockScreen {
    fn default() -> Self {
        Self::new()
    }
}

struct Layout {
    clock: Rect,
    avatar: Rect,
    label: Rect,
    field: Rect,
    status: Rect,
}

fn inset(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x + by,
        y: rect.y + by,
        width: (rect.width - by * 2.0).max(0.0),
        height: (rect.height - by * 2.0).max(0.0),
        radius: (rect.radius - by).max(0.0),
        color: rect.color,
    }
}

/// Scale `image` to cover `surface`, centred, the way
/// `Image.PreserveAspectCrop` does. Overhang falls outside the surface.
fn cover(image: &Image, surface: Rect) -> Image {
    let aspect = image.width as f32 / image.height.max(1) as f32;
    let (width, height) = if surface.width / surface.height.max(1.0) > aspect {
        (surface.width, surface.width / aspect)
    } else {
        (surface.height * aspect, surface.height)
    };
    Image {
        x: surface.x + (surface.width - width) * 0.5,
        y: surface.y + (surface.height - height) * 0.5,
        width_logical: width,
        height_logical: height,
        ..image.clone()
    }
}

/// The decoded wallpaper and avatar.
#[derive(Clone, Default)]
pub struct Artwork {
    pub backdrop: Option<Image>,
    pub avatar: Option<Image>,
}

/// Where the artwork comes from and how it is prepared.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtworkSpec {
    pub wallpaper: Option<PathBuf>,
    pub avatar: Option<PathBuf>,
    pub backdrop_width: u32,
    pub blur: f32,
    /// Avatar edge in pixels: its logical size times the largest output scale.
    pub avatar_px: u32,
    /// Modification times, so a changed wallpaper is decoded again.
    pub stamps: (Option<std::time::SystemTime>, Option<std::time::SystemTime>),
}

impl ArtworkSpec {
    pub fn new(
        config: &crate::config::LockConfig,
        metrics: &crate::ui::style::LockMetrics,
        scale: i32,
    ) -> Self {
        let wallpaper = expand_home(&config.wallpaper);
        let avatar = expand_home(&config.avatar);
        let stamp = |path: &Option<PathBuf>| {
            path.as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .and_then(|m| m.modified().ok())
        };
        let stamps = (stamp(&wallpaper), stamp(&avatar));
        Self {
            wallpaper,
            avatar,
            backdrop_width: metrics.backdrop_width.max(16.0) as u32,
            blur: metrics.backdrop_blur,
            avatar_px: (metrics.avatar * scale.max(1) as f32).round().max(1.0) as u32,
            stamps,
        }
    }

    /// Decode and prepare both images. Slow (a full-size PNG decode), so this
    /// runs on a worker thread.
    pub fn load(&self) -> Artwork {
        Artwork {
            backdrop: self
                .wallpaper
                .as_deref()
                .and_then(|path| load_backdrop(path, self.backdrop_width, self.blur)),
            avatar: self
                .avatar
                .as_deref()
                .and_then(|path| load_avatar(path, self.avatar_px)),
        }
    }
}

fn expand_home(path: &str) -> Option<PathBuf> {
    if path.trim().is_empty() {
        return None;
    }
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest)),
        None => Some(PathBuf::from(path)),
    }
}

/// Downscale, then blur. Blurring a small image and letting the GPU stretch it
/// with linear filtering looks the same as blurring at full size and costs a
/// tiny fraction of the time and atlas space.
fn load_backdrop(path: &Path, width: u32, blur: f32) -> Option<Image> {
    let decoded = image::open(path)
        .map_err(|err| tracing::warn!("lock wallpaper {}: {err}", path.display()))
        .ok()?;
    let height = ((decoded.height() as f32 / decoded.width().max(1) as f32) * width as f32)
        .round()
        .max(1.0) as u32;
    let small = image::imageops::resize(
        &decoded.to_rgba8(),
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    let blurred = if blur > 0.0 {
        image::imageops::fast_blur(&small, blur)
    } else {
        small
    };
    // The wallpaper is drawn opaque whatever its alpha.
    let mut pixels = blurred.into_raw();
    for px in pixels.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Some(Image {
        width,
        height,
        pixels: Arc::from(pixels),
        x: 0.0,
        y: 0.0,
        width_logical: width as f32,
        height_logical: height as f32,
    })
}

/// Crop to a centred square, scale, and cut out a circle with an
/// anti-aliased edge. The renderer has no rounded clip for images, so the
/// circle is baked into the pixels.
fn load_avatar(path: &Path, size: u32) -> Option<Image> {
    let decoded = image::open(path)
        .map_err(|err| tracing::warn!("lock avatar {}: {err}", path.display()))
        .ok()?;
    let side = decoded.width().min(decoded.height());
    let square = decoded.crop_imm(
        (decoded.width() - side) / 2,
        (decoded.height() - side) / 2,
        side,
        side,
    );
    let scaled = image::imageops::resize(
        &square.to_rgba8(),
        size,
        size,
        image::imageops::FilterType::Lanczos3,
    );
    let pixels = circle_mask(scaled.into_raw(), size);
    Some(Image {
        width: size,
        height: size,
        pixels: Arc::from(pixels),
        x: 0.0,
        y: 0.0,
        width_logical: size as f32,
        height_logical: size as f32,
    })
}

/// Apply a circular mask to straight-alpha RGBA and premultiply it.
fn circle_mask(mut pixels: Vec<u8>, size: u32) -> Vec<u8> {
    let radius = size as f32 * 0.5;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - radius;
            let dy = y as f32 + 0.5 - radius;
            let coverage = (radius - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let i = ((y * size + x) * 4) as usize;
            let alpha = pixels[i + 3] as f32 / 255.0 * coverage;
            for c in 0..3 {
                pixels[i + c] = (pixels[i + c] as f32 * alpha).round() as u8;
            }
            pixels[i + 3] = (alpha * 255.0).round() as u8;
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay_client_toolkit::seat::keyboard::Keysym;

    fn locked() -> LockScreen {
        let mut lock = LockScreen::new();
        assert!(lock.request());
        lock.confirm();
        lock
    }

    fn type_text(lock: &mut LockScreen, text: &str) {
        for ch in text.chars() {
            let s = ch.to_string();
            lock.key(Keysym::a, Some(&s), false);
        }
    }

    fn surface() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1600.0,
            height: 1000.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        }
    }

    #[test]
    fn a_second_request_does_not_lock_twice() {
        let mut lock = LockScreen::new();
        assert!(lock.request());
        assert!(!lock.request());
        lock.confirm();
        assert!(!lock.request());
        lock.finish();
        assert!(lock.request());
    }

    #[test]
    fn keys_before_confirmation_are_dropped() {
        let mut lock = LockScreen::new();
        lock.request();
        assert_eq!(lock.key(Keysym::a, Some("a"), false), KeyOutcome::Ignored);
        lock.confirm();
        assert_eq!(lock.key(Keysym::a, Some("a"), false), KeyOutcome::Changed);
    }

    #[test]
    fn enter_submits_and_holds_input_until_answered() {
        let mut lock = locked();
        type_text(&mut lock, "pw");
        let KeyOutcome::Submit(secret) = lock.key(Keysym::Return, None, false) else {
            panic!("expected a submit");
        };
        assert_eq!(secret.chars(), 2);
        // The field empties on submit, and typing waits for the answer.
        assert_eq!(lock.key(Keysym::a, Some("a"), false), KeyOutcome::Ignored);
        assert_eq!(lock.key(Keysym::Return, None, false), KeyOutcome::Ignored);
    }

    #[test]
    fn empty_enter_is_not_an_attempt() {
        let mut lock = locked();
        assert_eq!(lock.key(Keysym::Return, None, false), KeyOutcome::Ignored);
    }

    #[test]
    fn only_success_unlocks() {
        let mut lock = locked();
        type_text(&mut lock, "bad");
        lock.key(Keysym::Return, None, false);
        let failed = lock.auth_result(Err(crate::pam::AuthError {
            message: "Échec d'authentification".into(),
            wrong_password: true,
        }));
        assert!(!failed);
        assert_eq!(lock.phase(), Phase::Locked);
        assert_eq!(lock.status().as_deref(), Some("Incorrect password, try again."));

        // Retry is allowed, and the error clears as soon as the user types.
        type_text(&mut lock, "g");
        assert_eq!(lock.status(), None);
        lock.key(Keysym::Return, None, false);
        assert!(lock.auth_result(Ok(())));
    }

    #[test]
    fn specific_pam_reasons_are_shown() {
        let mut lock = locked();
        type_text(&mut lock, "x");
        lock.key(Keysym::Return, None, false);
        lock.auth_result(Err(crate::pam::AuthError {
            message: "Your account has expired".into(),
            wrong_password: false,
        }));
        assert_eq!(lock.status().as_deref(), Some("Your account has expired"));
    }

    /// Editing keys are the field's (tested in `password_field`); Escape is
    /// the lock's own: there is nothing to close, so it clears.
    #[test]
    fn escape_clears_the_field() {
        let mut lock = locked();
        type_text(&mut lock, "abc");
        assert_eq!(lock.key(Keysym::Escape, None, false), KeyOutcome::Changed);
        assert!(lock.field.is_empty());
        assert_eq!(lock.key(Keysym::Tab, Some("\t"), false), KeyOutcome::Ignored);
    }

    #[test]
    fn finishing_wipes_everything() {
        let mut lock = locked();
        type_text(&mut lock, "secret");
        lock.finish();
        assert!(lock.field.is_empty());
        assert_eq!(lock.phase(), Phase::Unlocked);
        assert!(!lock.active());
    }

    #[test]
    fn clock_repaints_once_per_minute() {
        let mut lock = locked();
        assert!(lock.clock_changed(100));
        assert!(!lock.clock_changed(100));
        assert!(lock.clock_changed(101));
    }

    #[test]
    fn column_is_centred_and_ordered() {
        let lock = locked();
        let s = surface();
        let l = lock.layout(s);
        assert!(l.clock.y < l.avatar.y && l.avatar.y < l.label.y);
        assert!(l.label.y < l.field.y && l.field.y < l.status.y);
        let top = l.clock.y - s.y;
        let bottom = s.y + s.height - (l.status.y + l.status.height);
        assert!((top - bottom).abs() < 0.5, "column is vertically centred");
        assert!((l.field.x + l.field.width * 0.5 - s.width * 0.5).abs() < 0.01);
    }

    #[test]
    fn cover_fills_the_surface_without_distortion() {
        let image = Image {
            width: 320,
            height: 180,
            pixels: Arc::from(vec![0u8; 320 * 180 * 4]),
            x: 0.0,
            y: 0.0,
            width_logical: 320.0,
            height_logical: 180.0,
        };
        // A 16:10 surface with a 16:9 image: height fills, width overhangs.
        let placed = cover(&image, surface());
        assert!((placed.height_logical - 1000.0).abs() < 0.01);
        assert!(placed.width_logical >= 1600.0);
        assert!((placed.width_logical / placed.height_logical - 320.0 / 180.0).abs() < 0.001);
        assert!(placed.x <= 0.0 && placed.x + placed.width_logical >= 1600.0);
    }

    #[test]
    fn avatar_mask_is_round() {
        let size = 32;
        let pixels = circle_mask(vec![255u8; (size * size * 4) as usize], size);
        let alpha = |x: u32, y: u32| pixels[((y * size + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0, "corners are cut");
        assert_eq!(alpha(16, 16), 255, "centre is solid");
        // Premultiplied: colour never exceeds alpha.
        for px in pixels.chunks_exact(4) {
            assert!(px[0] <= px[3]);
        }
    }

    #[test]
    fn home_is_expanded() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_home("~/a.png"),
            Some(PathBuf::from(home).join("a.png"))
        );
        assert_eq!(expand_home("/x.png"), Some(PathBuf::from("/x.png")));
        assert_eq!(expand_home(""), None);
    }
}

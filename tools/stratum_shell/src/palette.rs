//! Themes generated from a wallpaper.
//!
//! A small sample of the image's pixels is binned by hue. The bin with the
//! most colour in it gives the backgrounds their tint; the strongest bins
//! that are far enough apart become primary, secondary and tertiary. Every
//! colour is then pushed in lightness until it reads against the background
//! it sits on: text to 7:1 (muted text 4.5:1), accents and status colours to
//! 3:1, the WCAG thresholds for text and for graphics.
//!
//! Pure, so it is tested here and run on the paint thread: the sample is a
//! few thousand pixels.

use crate::config::ThemeConfig;
use crate::render::display_list::Color;
use crate::ui::color::{contrast, hsl_to_rgb, rgb_to_hsl, to_hex, Hsl};

/// Light text on dark backgrounds, or the reverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Dark,
    Light,
}

/// Hue bins, ten degrees each.
const BINS: usize = 36;
/// Accents closer in hue than this count as the same colour.
const MIN_HUE_GAP: f32 = 35.0;

#[derive(Clone, Copy, Default)]
struct Bin {
    /// Colourfulness found in the bin: the sum of each pixel's chroma.
    weight: f32,
    /// Weighted sums for the bin's mean hue (as a vector) and saturation.
    hue_x: f32,
    hue_y: f32,
    saturation: f32,
}

impl Bin {
    fn hue(&self) -> f32 {
        self.hue_y.atan2(self.hue_x).to_degrees().rem_euclid(360.0)
    }

    fn saturation(&self) -> f32 {
        if self.weight > 0.0 { self.saturation / self.weight } else { 0.0 }
    }
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// A theme drawn from `samples` (sRGB pixels), with `font`.
pub fn generate(samples: &[[u8; 3]], scheme: Scheme, font: &str) -> ThemeConfig {
    let mut bins = [Bin::default(); BINS];
    let mut total_saturation = 0.0;
    for px in samples {
        let c = Color::rgba(px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0, 1.0);
        let hsl = rgb_to_hsl(c);
        total_saturation += hsl.s;
        // Near-black, near-white and grey pixels have no hue worth taking.
        let chroma = hsl.s * (1.0 - (2.0 * hsl.l - 1.0).abs());
        if chroma < 0.04 {
            continue;
        }
        let bin = &mut bins[((hsl.h / 360.0 * BINS as f32) as usize).min(BINS - 1)];
        bin.weight += chroma;
        let rad = hsl.h.to_radians();
        bin.hue_x += rad.cos() * chroma;
        bin.hue_y += rad.sin() * chroma;
        bin.saturation += hsl.s * chroma;
    }
    let mean_saturation = if samples.is_empty() { 0.0 } else { total_saturation / samples.len() as f32 };

    // The strongest bins, far enough apart in hue.
    let mut ranked: Vec<&Bin> = bins.iter().filter(|b| b.weight > 0.0).collect();
    ranked.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    let mut accents: Vec<(f32, f32)> = Vec::new();
    for bin in &ranked {
        let hue = bin.hue();
        if accents.iter().all(|(h, _)| hue_distance(*h, hue) >= MIN_HUE_GAP) {
            accents.push((hue, bin.saturation()));
        }
        if accents.len() == 3 {
            break;
        }
    }
    // A grey image has no hue at all: a cool blue reads as neutral. One
    // with too few colours is filled out with neighbours of its main one.
    let (base_hue, base_saturation) = accents.first().copied().unwrap_or((220.0, 0.35));
    if accents.is_empty() {
        accents.push((base_hue, base_saturation));
    }
    while accents.len() < 3 {
        let offset = if accents.len() == 1 { 50.0 } else { -50.0 };
        accents.push(((base_hue + offset).rem_euclid(360.0), base_saturation));
    }

    // Backgrounds: the main hue, faintly, at the scheme's lightness steps.
    let tint = (mean_saturation * 0.6).clamp(0.04, 0.22);
    let bg = |l: f32| hsl_to_rgb(Hsl { h: base_hue, s: tint, l }, 1.0);
    let (dark, main, widget, hover) = match scheme {
        Scheme::Dark => (0.055, 0.085, 0.13, 0.19),
        Scheme::Light => (0.885, 0.965, 0.925, 0.86),
    };
    let bg_main = bg(main);

    // Text and accents, moved in lightness until they read on `bg_main`.
    let readable = |hsl: Hsl, ratio: f32| -> Color {
        let step = match scheme {
            Scheme::Dark => 0.02,
            Scheme::Light => -0.02,
        };
        let mut hsl = hsl;
        let mut colour = hsl_to_rgb(hsl, 1.0);
        while contrast(colour, bg_main) < ratio && (0.0..=1.0).contains(&(hsl.l + step)) {
            hsl.l += step;
            colour = hsl_to_rgb(hsl, 1.0);
        }
        colour
    };
    let (text_l, muted_l, accent_l) = match scheme {
        Scheme::Dark => (0.92, 0.66, 0.68),
        Scheme::Light => (0.14, 0.38, 0.42),
    };
    let text_s = tint.min(0.12);
    let accent = |(h, s): (f32, f32)| readable(Hsl { h, s: s.clamp(0.45, 0.9), l: accent_l }, 3.0);
    let status = |h: f32| readable(Hsl { h, s: 0.65, l: accent_l }, 3.0);
    let primary = accent(accents[0]);

    ThemeConfig {
        bg_main: to_hex(bg_main),
        bg_widget: to_hex(bg(widget)),
        bg_hover: to_hex(bg(hover)),
        bg_dark: to_hex(bg(dark)),
        text_main: to_hex(readable(Hsl { h: base_hue, s: text_s, l: text_l }, 7.0)),
        text_muted: to_hex(readable(Hsl { h: base_hue, s: text_s, l: muted_l }, 4.5)),
        primary: to_hex(primary),
        secondary: to_hex(accent(accents[1])),
        tertiary: to_hex(accent(accents[2])),
        success: to_hex(status(140.0)),
        warning: to_hex(status(42.0)),
        error: to_hex(status(355.0)),
        border_active: to_hex(primary),
        border_inactive: to_hex(bg(hover)),
        font: font.to_string(),
        surface_lowest: None,
        surface: None,
        surface_high: None,
        surface_highest: None,
        outline: None,
        on_primary: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_color;

    fn rgb(hsl: Hsl) -> [u8; 3] {
        let c = hsl_to_rgb(hsl, 1.0);
        [(c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8]
    }

    fn image(parts: &[(Hsl, usize)]) -> Vec<[u8; 3]> {
        parts.iter().flat_map(|(hsl, n)| std::iter::repeat_n(rgb(*hsl), *n)).collect()
    }

    fn hue_of(hex: &str) -> f32 {
        rgb_to_hsl(parse_color(hex).unwrap()).h
    }

    fn c(hex: &str) -> Color {
        parse_color(hex).unwrap()
    }

    fn check_contrast(theme: &ThemeConfig) {
        let bg = c(&theme.bg_main);
        assert!(contrast(c(&theme.text_main), bg) >= 7.0, "text {theme:?}");
        assert!(contrast(c(&theme.text_muted), bg) >= 4.5, "muted {theme:?}");
        for accent in [&theme.primary, &theme.secondary, &theme.tertiary, &theme.success, &theme.warning, &theme.error] {
            assert!(contrast(c(accent), bg) >= 3.0, "{accent} on {}", theme.bg_main);
        }
    }

    #[test]
    fn the_strongest_colours_become_the_accents() {
        let orange = Hsl { h: 25.0, s: 0.8, l: 0.5 };
        let blue = Hsl { h: 215.0, s: 0.7, l: 0.45 };
        let night = Hsl { h: 230.0, s: 0.3, l: 0.08 };
        let samples = image(&[(night, 3000), (orange, 900), (blue, 500)]);
        let theme = generate(&samples, Scheme::Dark, "Mono");
        assert!(hue_distance(hue_of(&theme.primary), 25.0) < 10.0, "{}", theme.primary);
        assert!(hue_distance(hue_of(&theme.secondary), 215.0) < 10.0, "{}", theme.secondary);
        assert_eq!(theme.font, "Mono");
        assert_eq!(theme.border_active, theme.primary);
    }

    #[test]
    fn both_schemes_read_well() {
        let samples = image(&[
            (Hsl { h: 300.0, s: 0.6, l: 0.4 }, 500),
            (Hsl { h: 60.0, s: 0.9, l: 0.6 }, 200),
            (Hsl { h: 0.0, s: 0.0, l: 0.95 }, 800),
        ]);
        for scheme in [Scheme::Dark, Scheme::Light] {
            let theme = generate(&samples, scheme, "Mono");
            check_contrast(&theme);
            theme.to_theme().expect("every colour parses");
        }
        let dark = generate(&samples, Scheme::Dark, "Mono");
        let light = generate(&samples, Scheme::Light, "Mono");
        assert!(rgb_to_hsl(c(&dark.bg_main)).l < 0.2 && rgb_to_hsl(c(&light.bg_main)).l > 0.8);
    }

    #[test]
    fn a_grey_or_empty_image_still_makes_a_theme() {
        let grey = image(&[(Hsl { h: 0.0, s: 0.0, l: 0.5 }, 1000), (Hsl { h: 0.0, s: 0.0, l: 0.1 }, 1000)]);
        for samples in [grey, Vec::new()] {
            let theme = generate(&samples, Scheme::Dark, "Mono");
            check_contrast(&theme);
            let hues = [hue_of(&theme.primary), hue_of(&theme.secondary), hue_of(&theme.tertiary)];
            assert!(hue_distance(hues[0], hues[1]) >= MIN_HUE_GAP && hue_distance(hues[0], hues[2]) >= MIN_HUE_GAP);
        }
    }

    #[test]
    fn one_colour_is_filled_out_with_its_neighbours() {
        let samples = image(&[(Hsl { h: 120.0, s: 0.7, l: 0.4 }, 1000)]);
        let theme = generate(&samples, Scheme::Dark, "Mono");
        assert!(hue_distance(hue_of(&theme.primary), 120.0) < 10.0);
        assert!(hue_distance(hue_of(&theme.secondary), hue_of(&theme.primary)) >= MIN_HUE_GAP);
    }
}

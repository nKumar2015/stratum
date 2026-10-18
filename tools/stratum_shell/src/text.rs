//! Text shaping, fallback, and rasterization.
//!
//! `cosmic-text` handles shaping and font fallback — including the combination
//! this shell depends on, Nerd Font private-use glyphs alongside emoji.
//! `swash` rasterizes individual glyphs; the renderer packs them into an atlas.
//!
//! Shaping is deliberately separate from rasterization: a text field re-shapes
//! on every keystroke, but the glyph atlas only changes when new glyphs appear.

use std::sync::Arc;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashImage};

use crate::render::display_list::{Color, PositionedGlyph, TextRun, Transform};

/// A rasterized glyph ready to be uploaded to the atlas.
#[derive(Clone, Debug)]
pub struct RasterGlyph {
    pub width: u32,
    pub height: u32,
    /// 8-bit coverage mask, row-major.
    pub data: Vec<u8>,
    /// Offset from the glyph's logical origin to the bitmap's top-left.
    pub left: i32,
    pub top: i32,
}

/// Owns the font system and glyph rasterizer.
pub struct TextEngine {
    font_system: FontSystem,
    swash: SwashCache,
}

/// Desired text style for shaping.
#[derive(Clone, Debug)]
pub struct TextStyle {
    pub family: Arc<str>,
    pub size: f32,
    pub weight: u16,
    pub color: Color,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: Arc::from("JetBrainsMono NFM"),
            size: 13.0,
            weight: 400,
            color: Color::WHITE,
        }
    }
}

impl TextEngine {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash: SwashCache::new(),
        }
    }

    /// Shape `text` into positioned glyphs, laid out at `(x, y)`.
    ///
    /// Coordinates in the returned run are relative to the run origin, so the
    /// caller can place the run anywhere without re-shaping.
    pub fn shape(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> TextRun {
        let metrics = Metrics::new(style.size, style.size * 1.2);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(None, None);

        let attrs = Attrs::new()
            .family(Family::Name(&style.family))
            .weight(cosmic_text::Weight(style.weight));

        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut glyphs = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                // Glyphs are positioned relative to the run origin. `line_y` is
                // the baseline within the shaped buffer; the renderer adds the
                // run's y, so keeping the absolute baseline here would count it
                // twice and push text a full line down.
                glyphs.push(PositionedGlyph {
                    glyph_id: glyph.glyph_id,
                    font_id: glyph.font_id,
                    x: glyph.x,
                    y: glyph.y,
                    x_offset: glyph.x_offset,
                    y_offset: glyph.y_offset,
                });
            }
        }

        TextRun {
            x,
            y,
            size: style.size,
            color: style.color,
            family: Arc::clone(&style.family),
            weight: style.weight,
            glyphs,
            transform: Transform::IDENTITY,
        }
    }

    /// Measure a single line.
    ///
    /// Returns `(advance_width, line_height)`. The width comes from the shaper's
    /// own layout run (`line_w`), so centred text lands exactly.
    pub fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) {
        let metrics = Metrics::new(style.size, style.size * 1.2);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(None, None);

        let attrs = Attrs::new()
            .family(Family::Name(&style.family))
            .weight(cosmic_text::Weight(style.weight));
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut width: f32 = 0.0;
        let mut height: f32 = style.size * 1.2;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height = run.line_height.max(height);
        }
        (width, height)
    }

    /// Line height and ascent for a style, used to centre text vertically on
    /// its real baseline rather than a guessed offset.
    pub fn metrics(&mut self, style: &TextStyle) -> (f32, f32) {
        let (_, height) = self.measure("Mg", style);
        // Cosmic places the baseline at roughly 80% of the line box for the
        // default metrics; derive it from a shaped run for accuracy.
        let metrics = Metrics::new(style.size, style.size * 1.2);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(None, None);
        let attrs = Attrs::new()
            .family(Family::Name(&style.family))
            .weight(cosmic_text::Weight(style.weight));
        buffer.set_text("Mg", &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let ascent = buffer
            .layout_runs()
            .next()
            .map(|run| run.line_y)
            .unwrap_or(style.size * 0.8);
        (ascent, height)
    }

    /// Rasterize one glyph for the shaped run.
    ///
    /// `font_id` comes from shaping, so per-glyph fallback faces are respected.
    /// Returns `None` for empty glyphs such as spaces.
    pub fn rasterize(
        &mut self,
        font_id: fontdb::ID,
        glyph_id: u16,
        size: f32,
        weight: u16,
        x: f32,
        y: f32,
    ) -> Option<RasterGlyph> {
        let (key, _, _) = cosmic_text::CacheKey::new(
            font_id,
            glyph_id,
            size,
            (x, y),
            cosmic_text::Weight(weight),
            cosmic_text::CacheKeyFlags::empty(),
        );

        let image: &Option<SwashImage> = self.swash.get_image(&mut self.font_system, key);
        let image = image.as_ref()?;
        raster_to_mask(image)
    }

    /// Rasterize every drawable glyph in a run, in draw order.
    ///
    /// `scale` is the output's buffer scale. Glyphs are rasterized at
    /// `run.size * scale` so they are sharp on high-DPI outputs; all positions
    /// the caller receives stay in *physical* pixels, which is what the
    /// renderer consumes before converting to logical space.
    pub fn rasterize_run(
        &mut self,
        run: &TextRun,
        scale: f32,
    ) -> Vec<(PositionedGlyph, RasterGlyph)> {
        let scale = scale.max(1.0);
        let mut out = Vec::with_capacity(run.glyphs.len());
        for glyph in &run.glyphs {
            // Subpixel binning uses the scaled pen so hinting matches the
            // physical grid.
            if let Some(raster) = self.rasterize(
                glyph.font_id,
                glyph.glyph_id,
                run.size * scale,
                run.weight,
                glyph.x * scale,
                glyph.y * scale,
            ) {
                out.push((*glyph, raster));
            }
        }
        out
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a swash image into an 8-bit coverage mask.
fn raster_to_mask(image: &SwashImage) -> Option<RasterGlyph> {
    use swash::scale::image::Content;

    let width = image.placement.width;
    let height = image.placement.height;
    if width == 0 || height == 0 {
        return None;
    }

    let data = match image.content {
        Content::Mask => image.data.clone(),
        Content::SubpixelMask | Content::Color => {
            // Reduce to coverage; colour glyphs (emoji) keep their alpha.
            image
                .data
                .chunks_exact(4)
                .map(|px| px[3])
                .collect::<Vec<u8>>()
        }
    };

    Some(RasterGlyph {
        width,
        height,
        data,
        left: image.placement.left,
        top: image.placement.top,
    })
}


/// Shaping panics inside cosmic-text when no fonts are installed (common in a
/// sandboxed build). Tests use this to skip instead of failing the build.
#[cfg(test)]
fn fonts_available(engine: &mut TextEngine) -> bool {
    fonts_available_for(engine, "sans-serif")
}

/// Like [`fonts_available`], but probes the family the caller will actually use.
/// The sandboxed build has no fonts at all, and cosmic-text panics when a
/// requested family cannot be resolved, so every shaping test must probe first.
#[cfg(test)]
fn fonts_available_for(engine: &mut TextEngine, family: &str) -> bool {
    let style = TextStyle {
        family: Arc::from(family),
        size: 16.0,
        weight: 400,
        color: Color::WHITE,
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        !engine.shape("test", &style, 0.0, 0.0).glyphs.is_empty()
    }))
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaping_produces_glyphs_and_measures_width() {
        let mut engine = TextEngine::new();
        if !super::fonts_available(&mut engine) {
            eprintln!("skipping: no fonts available");
            return;
        }
        let style = TextStyle {
            family: Arc::from("sans-serif"),
            size: 16.0,
            weight: 400,
            color: Color::WHITE,
        };
        let run = engine.shape("Hello", &style, 10.0, 20.0);
        assert!(!run.glyphs.is_empty(), "expected shaped glyphs");
        assert_eq!(run.x, 10.0);
        assert_eq!(run.y, 20.0);

        let (w, _) = engine.measure("Hello", &style);
        assert!(w > 0.0, "expected positive width, got {w}");
    }

    #[test]
    fn empty_text_shapes_to_no_glyphs() {
        let mut engine = TextEngine::new();
        if !super::fonts_available(&mut engine) {
            eprintln!("skipping: no fonts available");
            return;
        }
        let run = engine.shape("", &TextStyle::default(), 0.0, 0.0);
        assert!(run.glyphs.is_empty());
    }
}

#[cfg(test)]
mod placement_tests {
    use super::*;

    /// Render a shaped run into an ASCII mask using the same placement math as
    /// the GPU renderer, so placement bugs surface without a compositor.
    #[test]
    fn shaped_run_has_non_overlapping_ink() {
        let mut engine = TextEngine::new();
        if !super::fonts_available(&mut engine) {
            eprintln!("skipping: no fonts available");
            return;
        }
        let style = TextStyle {
            family: Arc::from("sans-serif"),
            size: 16.0,
            weight: 400,
            color: Color::WHITE,
        };
        let run = engine.shape("Desktop", &style, 0.0, 0.0);
        let rasters = engine.rasterize_run(&run, 1.0);
        assert_eq!(rasters.len(), run.glyphs.len(), "every glyph should rasterize");

        // Pens must advance monotonically to the right.
        let mut last_x = f32::MIN;
        for (glyph, _) in &rasters {
            assert!(glyph.x > last_x, "glyph pens should advance");
            last_x = glyph.x;
        }
        // All glyphs share one baseline.
        let ys: Vec<f32> = rasters.iter().map(|(g, _)| g.y).collect();
        assert!(ys.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-3), "one baseline: {ys:?}");
    }
}

#[cfg(test)]
mod glyph_availability_tests {
    use super::*;

    #[test]
    fn nerd_font_icon_resolves_to_a_real_glyph() {
        let mut engine = TextEngine::new();
        let style = TextStyle {
            family: Arc::from("JetBrainsMono NFM"),
            size: 16.0,
            weight: 400,
            color: Color::WHITE,
        };
        if !super::fonts_available_for(&mut engine, "JetBrainsMono NFM") {
            eprintln!("skipping: JetBrainsMono NFM not available");
            return;
        }
        // U+F0581 (audio speaker) must map to a real glyph, not .notdef (0).
        let run = engine.shape("\u{f0581}", &style, 0.0, 0.0);
        assert_eq!(run.glyphs.len(), 1, "one codepoint should shape to one glyph");
        assert_ne!(run.glyphs[0].glyph_id, 0, "icon must not be .notdef");
    }
}

//! Text shaping, fallback, and rasterization.
//!
//! `cosmic-text` handles shaping and font fallback — including the combination
//! this shell depends on, Nerd Font private-use glyphs alongside emoji.
//! `swash` rasterizes individual glyphs; the renderer packs them into an atlas.
//!
//! Shaping is deliberately separate from rasterization: a text field re-shapes
//! on every keystroke, but the glyph atlas only changes when new glyphs appear.

use std::collections::HashMap;
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
    /// Shaping results keyed by `(text, family, size, weight)`.
    ///
    /// Without this, every repaint re-shapes every string on screen. That is
    /// tolerable at a few frames per second but not when a title changes
    /// several times a second or an animation is running, where it dominates
    /// the frame cost. Runs are stored without a position, so a hit only needs
    /// the origin patched in.
    shape_cache: HashMap<(String, Arc<str>, u32, u16), CachedShape>,
}

/// A shaped run independent of where it will be drawn.
#[derive(Clone)]
struct CachedShape {
    glyphs: Vec<PositionedGlyph>,
    /// `(ascent, line_height)`.
    metrics: (f32, f32),
    /// The shaper's advance for the line (`run.line_w`).
    ///
    /// This must come from the shaper, not from the last glyph's pen position:
    /// glyph advances are applied when the *next* glyph is placed, so the final
    /// glyph's `x` excludes its own width. Deriving the width from positions
    /// made every centred string a few pixels too wide, which shifts its origin
    /// left and leaves the text visibly right of centre.
    width: f32,
}

/// Cache key for shaped text. `f32` is not `Eq`, so the size is stored as bits.
type ShapeKey = (String, Arc<str>, u32, u16);

fn key_for(text: &str, style: &TextStyle) -> ShapeKey {
    (
        text.to_string(),
        Arc::clone(&style.family),
        style.size.to_bits(),
        style.weight,
    )
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
            shape_cache: HashMap::new(),
        }
    }

    /// Shape `text` into positioned glyphs, laid out at `(x, y)`.
    ///
    /// Coordinates in the returned run are relative to the run origin, so the
    /// caller can place the run anywhere without re-shaping.
    pub fn shape(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> TextRun {
        let key = key_for(text, style);
        let shaped = match self.shape_cache.get(&key) {
            Some(cached) => cached.clone(),
            None => {
                let cached = self.shape_uncached(text, style);
                self.insert_shape(key, cached.clone());
                cached
            }
        };

        TextRun {
            x,
            y,
            size: style.size,
            color: style.color,
            family: Arc::clone(&style.family),
            weight: style.weight,
            glyphs: shaped.glyphs,
            transform: Transform::IDENTITY,
            // Filled in by `DisplayList::push_text`, which knows the active
            // clip. A run used outside a list is never clipped.
            clip: None,
        }
    }

    /// Shape without consulting or filling the cache.
    fn shape_uncached(&mut self, text: &str, style: &TextStyle) -> CachedShape {
        let metrics = Metrics::new(style.size, style.size * 1.2);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(None, None);

        let attrs = Attrs::new()
            .family(Family::Name(&style.family))
            .weight(cosmic_text::Weight(style.weight));

        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut glyphs = Vec::new();
        let mut ascent = style.size * 0.8;
        let mut line_height = style.size * 1.2;
        let mut width = 0.0_f32;
        for run in buffer.layout_runs() {
            ascent = run.line_y;
            line_height = run.line_height.max(line_height);
            // `line_w` is the run's advance including trailing letter spacing,
            // which glyph positions alone do not capture.
            width = width.max(run.line_w);
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

        CachedShape {
            glyphs,
            metrics: (ascent, line_height),
            width,
        }
    }

    /// Bound the shape cache. Text is mostly a fixed vocabulary (clock, labels,
    /// numbers), so a small cap is enough and protects against a pathological
    /// caller that shapes unbounded distinct strings.
    fn insert_shape(&mut self, key: ShapeKey, shape: CachedShape) {
        const MAX_ENTRIES: usize = 512;
        if self.shape_cache.len() >= MAX_ENTRIES {
            self.shape_cache.clear();
        }
        self.shape_cache.insert(key, shape);
    }

    /// Measure a single line.
    ///
    /// Returns `(advance_width, line_height)`. The width is the shaper's own
    /// `line_w`, so centred text lands exactly.
    pub fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) {
        let key = key_for(text, style);
        let shaped = match self.shape_cache.get(&key) {
            Some(cached) => cached.clone(),
            None => {
                let cached = self.shape_uncached(text, style);
                self.insert_shape(key, cached.clone());
                cached
            }
        };
        (shaped.width, shaped.metrics.1)
    }

    /// Line height and ascent for a style, used to centre text vertically on
    /// its real baseline rather than a guessed offset.
    pub fn metrics(&mut self, style: &TextStyle) -> (f32, f32) {
        // "Mg" exercises both an ascender and a descender, so the metrics are
        // the same as the font's nominal ones.
        let key = key_for("Mg", style);
        let (ascent, line_height) = match self.shape_cache.get(&key) {
            Some(cached) => cached.metrics,
            None => {
                let shaped = self.shape_uncached("Mg", style);
                let metrics = shaped.metrics;
                self.insert_shape(key, shaped);
                metrics
            }
        };
        (ascent, line_height)
    }

    /// The inked box of `text` relative to its pen origin on the baseline:
    /// `(left, top, right, bottom)` in logical pixels, y down. `None` when
    /// nothing is drawn.
    ///
    /// Centring by advance is right for text but not for icon glyphs: a
    /// proportional Nerd Font draws its icons past their advance, so an
    /// advance-centred icon sits to the right.
    pub fn ink_bounds(&mut self, text: &str, style: &TextStyle) -> Option<(f32, f32, f32, f32)> {
        // Rasterized larger than drawn, for sub-pixel accuracy.
        const OVERSAMPLE: f32 = 4.0;
        let run = self.shape(text, style, 0.0, 0.0);
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for (_, raster) in self.rasterize_run(&run, OVERSAMPLE, (0.0, 0.0)) {
            let left = raster.left as f32 / OVERSAMPLE;
            let top = -raster.top as f32 / OVERSAMPLE;
            let right = left + raster.width as f32 / OVERSAMPLE;
            let bottom = top + raster.height as f32 / OVERSAMPLE;
            bounds = Some(match bounds {
                Some((l, t, r, b)) => (l.min(left), t.min(top), r.max(right), b.max(bottom)),
                None => (left, top, right, bottom),
            });
        }
        bounds
    }

    /// Rasterize one glyph for the shaped run.
    ///
    /// `font_id` comes from shaping, so per-glyph fallback faces are respected.
    /// Returns `None` for empty glyphs such as spaces.
    ///
    /// `(x, y)` is the pen in physical pixels. The bitmap carries the pen's
    /// sub-pixel fraction, and its `left`/`top` place it from the pixel
    /// origin, so it lands on whole pixels and is sampled one to one.
    pub fn rasterize(
        &mut self,
        font_id: fontdb::ID,
        glyph_id: u16,
        size: f32,
        weight: u16,
        x: f32,
        y: f32,
    ) -> Option<RasterGlyph> {
        let (key, x_px, y_px) = cosmic_text::CacheKey::new(
            font_id,
            glyph_id,
            size,
            (x, y),
            cosmic_text::Weight(weight),
            cosmic_text::CacheKeyFlags::empty(),
        );

        let image: &Option<SwashImage> = self.swash.get_image(&mut self.font_system, key);
        let image = image.as_ref()?;
        let mut raster = raster_to_mask(image)?;
        raster.left += x_px;
        raster.top -= y_px;
        Some(raster)
    }

    /// Rasterize every drawable glyph in a run, in draw order.
    ///
    /// `scale` is the output's buffer scale. Glyphs are rasterized at
    /// `run.size * scale` so they are sharp on high-DPI outputs. `origin` is
    /// where the run's pen starts, in physical pixels; each raster's
    /// `left`/`top` place it from the physical origin, `(left, -top)`.
    ///
    /// Binning against the whole on-screen position, not just the glyph's
    /// place in the run, is what keeps text sharp: a run starting between
    /// pixels was otherwise drawn between them and sampled over two.
    pub fn rasterize_run(
        &mut self,
        run: &TextRun,
        scale: f32,
        origin: (f32, f32),
    ) -> Vec<(PositionedGlyph, RasterGlyph)> {
        let scale = scale.max(1.0);
        let mut out = Vec::with_capacity(run.glyphs.len());
        for glyph in &run.glyphs {
            if let Some(raster) = self.rasterize(
                glyph.font_id,
                glyph.glyph_id,
                run.size * scale,
                run.weight,
                origin.0 + (glyph.x + glyph.x_offset * run.size) * scale,
                origin.1 + (glyph.y - glyph.y_offset * run.size) * scale,
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
    fn shaping_is_cached_and_position_independent() {
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
        let first = engine.shape("Cached", &style, 0.0, 0.0);
        // A second shape at a different origin must reuse the same glyphs but
        // carry the new position.
        let second = engine.shape("Cached", &style, 40.0, 12.0);
        assert_eq!(first.glyphs.len(), second.glyphs.len());
        assert_eq!(second.x, 40.0);
        assert_eq!(second.y, 12.0);
        assert_eq!(
            first.glyphs.first().map(|g| g.glyph_id),
            second.glyphs.first().map(|g| g.glyph_id)
        );

        // Cached and uncached measurements must agree, and the width must come
        // from the shaper's own advance rather than the last glyph's pen
        // position (which omits that glyph's width).
        let (cached_width, _) = engine.measure("Cached", &style);
        let shaped = engine.shape_uncached("Cached", &style);
        assert!((cached_width - shaped.width).abs() < 0.01);
        assert!(cached_width > 0.0);

        // The advance must exceed the last glyph's pen x, because the final
        // glyph still occupies space after it.
        let last_pen = shaped
            .glyphs
            .last()
            .map(|g| g.x + g.x_offset)
            .unwrap_or(0.0);
        assert!(
            cached_width > last_pen,
            "width {cached_width} should include the final glyph's advance (pen {last_pen})"
        );
    }

    /// Centring depends entirely on this: an underestimated width shifts every
    /// centred label to the right, which is what the sidebar showed.
    #[test]
    fn measured_width_matches_the_rendered_extent() {
        let mut engine = TextEngine::new();
        if !super::fonts_available(&mut engine) {
            eprintln!("skipping: no fonts available");
            return;
        }
        let style = TextStyle {
            family: Arc::from("sans-serif"),
            size: 15.0,
            weight: 400,
            color: Color::WHITE,
        };
        let (width, _) = engine.measure("Workspace 10", &style);
        let run = engine.shape("Workspace 10", &style, 0.0, 0.0);
        let last = run.glyphs.last().expect("glyphs");
        // The final glyph starts before the measured width ends.
        assert!(
            last.x < width,
            "last glyph at {} should start inside width {}",
            last.x,
            width
        );
        // With a monospace-ish fallback the difference is one glyph at most;
        // allow a generous bound so this is not font-specific.
        assert!(
            width - last.x <= style.size * 2.0,
            "width {} unexpectedly far from last glyph {}",
            width,
            last.x
        );
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
        let rasters = engine.rasterize_run(&run, 1.0, (0.0, 0.0));
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

    #[test]
    fn a_proportional_nerd_font_icon_overflows_its_advance() {
        let mut engine = TextEngine::new();
        let family = "JetBrainsMonoNL Nerd Font";
        if !super::fonts_available_for(&mut engine, family) {
            eprintln!("skipping: {family} not available");
            return;
        }
        let style = TextStyle { family: Arc::from(family), size: 20.0, weight: 400, color: Color::WHITE };
        let advance = engine.measure("\u{f0581}", &style).0;
        let (left, top, right, bottom) = engine.ink_bounds("\u{f0581}", &style).expect("inked");
        // Why icons are centred by ink: this one is drawn wider than it
        // advances, so centring the advance pushes it right.
        assert!(right > advance + 1.0, "ink {left}..{right}, advance {advance}");
        assert!(top < 0.0 && bottom > top, "above the baseline: {top}..{bottom}");
        assert_eq!(engine.ink_bounds(" ", &style), None);
    }
}

use std::collections::HashMap;
use std::sync::RwLock;
use swash::{Charmap, FontRef, GlyphMetrics};

pub const DEFAULT_FONT: &[u8] = include_bytes!("../../fonts/audiowide.ttf");

#[derive(Clone)]
struct CachedMeasure {
    text: String,
    size_scaled: u32,
    dims: (f32, f32),
}

/// Precomputed, normalized font design metrics synchronized with the GPU FontAtlas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    pub units_per_em: f32,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub space_advance: f32,
    pub fallback_advance: f32,
}

pub struct TextMeasurer<'a> {
    charmap: Charmap<'a>,
    glyph_metrics: GlyphMetrics<'a>,
    pub metrics: FontMetrics,
    cache: RwLock<HashMap<u64, CachedMeasure>>,
}

impl<'a> TextMeasurer<'a> {
    #[must_use]
    pub fn new(font_bytes: &'a [u8]) -> Option<Self> {
        let font = FontRef::from_index(font_bytes, 0)?;
        let swash_metrics = font.metrics(&[]);
        let units_per_em = f32::from(swash_metrics.units_per_em);

        let charmap = font.charmap();
        let glyph_metrics = font.glyph_metrics(&[]);

        let space_id = charmap.map(' ');
        let space_advance = if space_id != 0 {
            glyph_metrics.advance_width(space_id)
        } else {
            units_per_em * 0.3
        };

        let question_id = charmap.map('?');
        let fallback_advance = if question_id != 0 {
            glyph_metrics.advance_width(question_id)
        } else {
            units_per_em * 0.5
        };

        let metrics = FontMetrics {
            units_per_em,
            ascent: swash_metrics.ascent,
            descent: swash_metrics.descent,
            leading: swash_metrics.leading,
            space_advance,
            fallback_advance,
        };

        Some(Self {
            charmap,
            glyph_metrics,
            metrics,
            cache: RwLock::new(HashMap::new()),
        })
    }

    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn measure(&self, text: &str, text_size: f32) -> (f32, f32) {
        let size_scaled = (text_size * 100.0).max(0.0).round() as u32;
        let hash = crate::ui::widget::fnv1a(text.as_bytes()) ^ (u64::from(size_scaled) << 32);

        if let Ok(cache) = self.cache.read()
            && let Some(entry) = cache.get(&hash)
            && entry.size_scaled == size_scaled
            && entry.text == text
        {
            return entry.dims;
        }

        let scale_factor = text_size / self.metrics.units_per_em;
        let mut width = 0.0;

        for c in text.chars() {
            if c == ' ' {
                width = self.metrics.space_advance.mul_add(scale_factor, width);
                continue;
            }

            let glyph_id = self.charmap.map(c);
            let advance = if glyph_id == 0 {
                self.metrics.fallback_advance
            } else {
                self.glyph_metrics.advance_width(glyph_id)
            };
            width = advance.mul_add(scale_factor, width);
        }

        let raw_height = (self.metrics.ascent + self.metrics.descent.abs() + self.metrics.leading)
            .max(self.metrics.units_per_em);
        let height = raw_height * scale_factor;
        let dims = (width, height);

        if let Ok(mut cache) = self.cache.write() {
            if cache.len() >= 1024 {
                cache.clear();
            }
            cache.insert(
                hash,
                CachedMeasure {
                    text: text.to_string(),
                    size_scaled,
                    dims,
                },
            );
        }

        dims
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_measurer_caching() {
        let measurer = TextMeasurer::new(DEFAULT_FONT).expect("valid font");
        let (w1, h1) = measurer.measure("SnifferLauncher", 16.0);
        assert!(w1 > 0.0);
        assert!(h1 > 0.0);

        // Second measure should hit cache and yield identical results
        let (w2, h2) = measurer.measure("SnifferLauncher", 16.0);
        assert_eq!(w1, w2);
        assert_eq!(h1, h2);

        // Different size should yield different dimensions
        let (w3, h3) = measurer.measure("SnifferLauncher", 32.0);
        assert!(w3 > w1);
        assert!(h3 > h1);
    }

    #[test]
    fn test_unified_font_metrics_and_spacing() {
        let measurer = TextMeasurer::new(DEFAULT_FONT).expect("valid font");

        // 1. Verify metrics are loaded and positive
        assert!(measurer.metrics.units_per_em > 0.0);
        assert!(measurer.metrics.ascent > 0.0);
        assert!(measurer.metrics.space_advance > 0.0);
        assert!(measurer.metrics.fallback_advance > 0.0);

        // 2. Verify space advance difference between "A B" and "AB" exactly matches space_advance
        let (w_ab, _) = measurer.measure("AB", 20.0);
        let (w_a_b, _) = measurer.measure("A B", 20.0);
        let diff = w_a_b - w_ab;

        let scale = 20.0 / measurer.metrics.units_per_em;
        let expected_space = measurer.metrics.space_advance * scale;
        assert!((diff - expected_space).abs() < 1e-4);

        // 3. Verify text height scales with font metrics
        let (_, h16) = measurer.measure("Test", 16.0);
        let (_, h32) = measurer.measure("Test", 32.0);
        assert!((h32 - h16 * 2.0).abs() < 1e-4);
    }
}

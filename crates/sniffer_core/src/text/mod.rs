use std::collections::hash_map::{Entry, HashMap};
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
    cache: RwLock<MeasureCache>,
}

/// Fixed-capacity circular ring cache for text measurements.
/// Prevents cache stampede / frame drops by evicting only the oldest entry when full.
pub struct MeasureCache {
    map: HashMap<u64, CachedMeasure>,
    ring: Vec<u64>,
    head: usize,
    capacity: usize,
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
            cache: RwLock::new(MeasureCache::new(1024)),
        })
    }

    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn measure(&self, text: &str, text_size: f32) -> (f32, f32) {
        let size_scaled = (text_size * 100.0).max(0.0).round() as u32;
        let hash = crate::ui::widget::fnv1a(text.as_bytes()) ^ (u64::from(size_scaled) << 32);

        if let Ok(cache) = self.cache.read() {
            if let Some(dims) = cache.get(hash, text, size_scaled) {
                return dims;
            }
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
            cache.insert(hash, text.to_string(), size_scaled, dims);
        }

        dims
    }
}

impl MeasureCache {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            ring: Vec::with_capacity(capacity),
            head: 0,
            capacity,
        }
    }

    #[must_use]
    pub fn get(&self, hash: u64, text: &str, size_scaled: u32) -> Option<(f32, f32)> {
        if let Some(entry) = self.map.get(&hash) {
            if entry.size_scaled == size_scaled && entry.text == text {
                return Some(entry.dims);
            }
        }
        None
    }

    pub fn insert(&mut self, hash: u64, text: String, size_scaled: u32, dims: (f32, f32)) {
        if let Entry::Occupied(mut e) = self.map.entry(hash) {
            e.insert(CachedMeasure {
                text,
                size_scaled,
                dims,
            });
            return;
        }

        // If capacity is not yet reached, append key to the ring
        if self.ring.len() < self.capacity {
            self.ring.push(hash);
        } else {
            // When full: evict only the oldest entry and overwrite slot in circular buffer
            let oldest_hash = self.ring[self.head];
            self.map.remove(&oldest_hash);
            self.ring[self.head] = hash;
            self.head = (self.head + 1) % self.capacity;
        }

        self.map.insert(
            hash,
            CachedMeasure {
                text,
                size_scaled,
                dims,
            },
        );
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
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

    #[test]
    fn test_measure_cache_circular_eviction() {
        let mut cache = MeasureCache::new(4);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);

        // Insert 4 items to fill the cache
        for i in 0..4 {
            let key = format!("item_{i}");
            cache.insert(i as u64, key, 1600, (10.0 * i as f32, 20.0));
        }
        assert_eq!(cache.len(), 4);

        // Verify all 4 items exist
        for i in 0..4 {
            let key = format!("item_{i}");
            assert!(cache.get(i as u64, &key, 1600).is_some());
        }

        // Insert 5th item: item_0 (oldest) must be evicted, capacity stays 4
        cache.insert(4, "item_4".to_string(), 1600, (40.0, 20.0));
        assert_eq!(cache.len(), 4);
        assert!(cache.get(0, "item_0", 1600).is_none());
        assert!(cache.get(4, "item_4", 1600).is_some());

        // Insert 6th item: item_1 must be evicted
        cache.insert(5, "item_5".to_string(), 1600, (50.0, 20.0));
        assert_eq!(cache.len(), 4);
        assert!(cache.get(1, "item_1", 1600).is_none());
        assert!(cache.get(2, "item_2", 1600).is_some());
        assert!(cache.get(3, "item_3", 1600).is_some());
        assert!(cache.get(4, "item_4", 1600).is_some());
        assert!(cache.get(5, "item_5", 1600).is_some());

        // Update existing item_2: should overwrite without changing length
        cache.insert(2, "item_2".to_string(), 1600, (99.0, 99.0));
        assert_eq!(cache.len(), 4);
        assert_eq!(cache.get(2, "item_2", 1600), Some((99.0, 99.0)));
    }
}

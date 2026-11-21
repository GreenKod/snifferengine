use swash::FontRef;
use swash::scale::{Render, ScaleContext, Source, StrikeWith, image::Content};

/// Sentinel for "no seed found yet" in the distance field.
const INF_DIST: i32 = 0x7FFF;

/// Displacement vector from a pixel to its nearest seed in the 8-SED
/// distance field. `dx`/`dy` are in padded pixels; `dx * dx + dy * dy`
/// is the squared Euclidean distance.
#[derive(Clone, Copy)]
struct DistPixel {
    dx: i32,
    dy: i32,
}

impl DistPixel {
    #[inline]
    fn dist_sq(&self) -> i64 {
        i64::from(self.dx) * i64::from(self.dx) + i64::from(self.dy) * i64::from(self.dy)
    }
}

/// Tries to improve `best` by adopting a neighbor's seed offset by
/// `(offset_dx, offset_dy)`.
#[inline]
fn compare_neighbor(
    best: &mut DistPixel,
    neighbor: Option<DistPixel>,
    offset_dx: i32,
    offset_dy: i32,
) {
    if let Some(n) = neighbor {
        let cand = DistPixel {
            dx: n.dx + offset_dx,
            dy: n.dy + offset_dy,
        };
        if cand.dist_sq() < best.dist_sq() {
            *best = cand;
        }
    }
}

/// Forward pass of the 8-point Sequential Euclidean Distance Transform:
/// scans top-left to bottom-right, propagating seed information from
/// the left, up, up-left, and up-right neighbors.
fn sed_forward_pass(dist: &mut [DistPixel], width: usize, height: usize) {
    for y in 0..height {
        let row = y * width;
        for x in 0..width {
            let idx = row + x;
            let mut best = dist[idx];

            // Left neighbor: one step further from its seed.
            if x > 0 {
                compare_neighbor(&mut best, Some(dist[idx - 1]), 1, 0);
            }
            // Up neighbor.
            if y > 0 {
                compare_neighbor(&mut best, Some(dist[idx - width]), 0, 1);
                // Up-left diagonal.
                if x > 0 {
                    compare_neighbor(&mut best, Some(dist[idx - width - 1]), 1, 1);
                }
                // Up-right diagonal.
                if x < width - 1 {
                    compare_neighbor(&mut best, Some(dist[idx - width + 1]), -1, 1);
                }
            }

            dist[idx] = best;
        }
    }
}

/// Backward pass of the 8-SED: scans bottom-right to top-left,
/// propagating seed information from the right, down, down-right, and
/// down-left neighbors.
fn sed_backward_pass(dist: &mut [DistPixel], width: usize, height: usize) {
    for y in (0..height).rev() {
        let row = y * width;
        for x in (0..width).rev() {
            let idx = row + x;
            let mut best = dist[idx];

            // Right neighbor.
            if x < width - 1 {
                compare_neighbor(&mut best, Some(dist[idx + 1]), -1, 0);
            }
            // Down neighbor.
            if y < height - 1 {
                compare_neighbor(&mut best, Some(dist[idx + width]), 0, -1);
                // Down-right diagonal.
                if x < width - 1 {
                    compare_neighbor(&mut best, Some(dist[idx + width + 1]), -1, -1);
                }
                // Down-left diagonal.
                if x > 0 {
                    compare_neighbor(&mut best, Some(dist[idx + width - 1]), 1, -1);
                }
            }

            dist[idx] = best;
        }
    }
}

/// Computes one 8-SED distance field from `padded` where `is_seed_fn`
/// decides which pixels are seeds (distance 0) for this field.
fn compute_sed_field(
    padded: &[u8],
    width: usize,
    height: usize,
    is_seed: impl Fn(u8) -> bool,
) -> Vec<DistPixel> {
    let len = width * height;
    let mut dist = vec![
        DistPixel {
            dx: INF_DIST,
            dy: INF_DIST,
        };
        len
    ];

    // Seeds have zero distance; everything else starts unreachable.
    for (idx, &v) in padded.iter().enumerate().take(len) {
        if is_seed(v) {
            dist[idx] = DistPixel { dx: 0, dy: 0 };
        }
    }

    sed_forward_pass(&mut dist, width, height);
    sed_backward_pass(&mut dist, width, height);
    dist
}

pub struct GlyphData {
    pub width: u32,
    pub height: u32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance_width: f32,
    pub is_color: bool,
    pub bitmap: Vec<u8>,
}

thread_local! {
    /// Reusable swash `ScaleContext` — caches font data across glyph
    /// renders instead of reallocating per character (Stage 3.4).
    static SCALE_CONTEXT: std::cell::RefCell<ScaleContext> =
        std::cell::RefCell::new(ScaleContext::new());
}

pub fn render_character(
    character: char,
    font_bytes_list: &[Vec<u8>],
    pixel_size: f32,
) -> Option<GlyphData> {
    for font_bytes in font_bytes_list {
        if let Some(font) = FontRef::from_index(font_bytes, 0) {
            let charmap = font.charmap();
            let glyph_id = charmap.map(character);
            if glyph_id == 0 {
                continue;
            }

            let font_metrics = font.metrics(&[]);
            let units_per_em = f32::from(font_metrics.units_per_em);
            let scale_factor = pixel_size / units_per_em;
            let glyph_metrics = font.glyph_metrics(&[]);
            let advance_width = glyph_metrics.advance_width(glyph_id) * scale_factor;

            // Reuse the cached ScaleContext across glyphs: swash keeps
            // internal font data caches that make repeated scaler builds
            // far cheaper than a fresh context per character.
            let image = SCALE_CONTEXT.with_borrow_mut(|context| {
                let mut scaler = context.builder(font).size(pixel_size).hint(true).build();

                let renderer = Render::new(&[
                    Source::ColorBitmap(StrikeWith::BestFit),
                    Source::ColorOutline(0),
                    Source::Outline,
                    Source::Bitmap(StrikeWith::BestFit),
                ]);

                renderer.render(&mut scaler, glyph_id)
            });

            if let Some(img) = image {
                let w = img.placement.width;
                let h = img.placement.height;
                #[allow(clippy::cast_precision_loss)]
                let bx = img.placement.left as f32;
                #[allow(clippy::cast_precision_loss)]
                let by = img.placement.top as f32;

                match img.content {
                    Content::Color => {
                        return Some(GlyphData {
                            width: w,
                            height: h,
                            bearing_x: bx,
                            bearing_y: by,
                            advance_width,
                            is_color: true,
                            bitmap: img.data,
                        });
                    }
                    Content::SubpixelMask => {
                        let mut gray = Vec::with_capacity(img.data.len() / 3);
                        for chunk in img.data.as_chunks::<3>().0 {
                            let g =
                                (u32::from(chunk[0]) + u32::from(chunk[1]) + u32::from(chunk[2]))
                                    / 3;
                            #[allow(clippy::cast_possible_truncation)]
                            gray.push(g as u8);
                        }
                        let spread = 8;
                        let (sdf_bitmap, sdf_w, sdf_h) = generate_sdf(&gray, w, h, spread);
                        return Some(GlyphData {
                            width: sdf_w,
                            height: sdf_h,
                            bearing_x: bx - spread as f32,
                            bearing_y: by + spread as f32,
                            advance_width,
                            is_color: false,
                            bitmap: sdf_bitmap,
                        });
                    }
                    Content::Mask => {
                        let spread = 8;
                        let (sdf_bitmap, sdf_w, sdf_h) = generate_sdf(&img.data, w, h, spread);
                        return Some(GlyphData {
                            width: sdf_w,
                            height: sdf_h,
                            bearing_x: bx - spread as f32,
                            bearing_y: by + spread as f32,
                            advance_width,
                            is_color: false,
                            bitmap: sdf_bitmap,
                        });
                    }
                }
            }
        }
    }

    None
}

/// Generates a Signed Distance Field using the 8-point Sequential
/// Euclidean Distance Transform (8-SED / 8SSEDT).
///
/// Two linear O(W·H) passes replace the brute-force O(W·H·S²)
/// neighborhood scan: a forward pass (top-left to bottom-right) and a
/// backward pass (bottom-right to top-left), each consulting four
/// already-processed neighbors. For a typical 48×48 padded glyph with
/// spread 8, the brute-force scan paid ~666K comparisons per glyph;
/// 8-SED pays ~18K — roughly 37× fewer.
pub fn generate_sdf(bitmap: &[u8], width: u32, height: u32, spread: u32) -> (Vec<u8>, u32, u32) {
    if width == 0 || height == 0 {
        return (Vec::new(), 0, 0);
    }

    let p_width = width + 2 * spread;
    let p_height = height + 2 * spread;
    let pw = p_width as usize;
    let ph = p_height as usize;
    let mut padded = vec![0u8; pw * ph];

    for y in 0..height as usize {
        for x in 0..width as usize {
            let src = y * width as usize + x;
            let dst = (y + spread as usize) * pw + (x + spread as usize);
            padded[dst] = bitmap[src];
        }
    }

    // Two independent distance fields:
    // - outside_field: distance to the nearest outside pixel (used by
    //   inside pixels for their positive distance).
    // - inside_field: distance to the nearest inside pixel (used by
    //   outside pixels for their negative distance).
    let outside_field = compute_sed_field(&padded, pw, ph, |v| v <= 127);
    let inside_field = compute_sed_field(&padded, pw, ph, |v| v > 127);

    let spread_f = spread as f32;
    let mut sdf = vec![0u8; pw * ph];

    for idx in 0..pw * ph {
        let is_inside = padded[idx] > 127;
        let dist = if is_inside {
            let d = outside_field[idx];
            (d.dist_sq() as f32).sqrt()
        } else {
            let d = inside_field[idx];
            -((d.dist_sq() as f32).sqrt())
        };

        let norm = 0.5 + 0.5 * (dist / spread_f);
        let val = (norm * 255.0).clamp(0.0, 255.0) as u8;
        sdf[idx] = val;
    }

    (sdf, p_width, p_height)
}

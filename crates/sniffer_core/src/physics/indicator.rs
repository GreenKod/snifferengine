/// Computes geometry and interpolated colors for pager indicator dots.
///
/// The fractional `page_float` supplied by the physics pipeline drives dot
/// pill stretching and tinting at draw time, without any tree mutation.
#[must_use]
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
pub fn compute_indicator_dots(
    count: usize,
    page_float: f32,
    vmin_px: f32,
    vertical: bool,
    container_rect: crate::Rect,
) -> Vec<(crate::Rect, u32)> {
    if count == 0 {
        return Vec::new();
    }
    let gap = 1.2 * vmin_px;
    let dot_base = 1.6 * vmin_px;
    let mut results = Vec::with_capacity(count);

    if vertical {
        let mut total_h = 0.0_f32;
        let mut dot_heights = Vec::with_capacity(count);
        for p in 0..count {
            let dist = (page_float - p as f32).abs();
            let raw_activity = (1.0 - dist).clamp(0.0, 1.0);
            let activity = raw_activity * raw_activity * (3.0 - 2.0 * raw_activity);
            let h = (1.6 + 2.0 * activity) * vmin_px;
            dot_heights.push((h, activity));
            total_h += h;
        }
        total_h += (count.saturating_sub(1) as f32) * gap;

        let center_x = container_rect.x + (container_rect.width - dot_base) * 0.5;
        let mut cur_y = container_rect.y + (container_rect.height - total_h) * 0.5;
        for (h, activity) in dot_heights {
            let rect = crate::Rect::new(center_x, cur_y, dot_base, h);
            cur_y += h + gap;

            let a = (0x55 as f32 + (0xFF - 0x55) as f32 * activity).round() as u32;
            let r = (0xFF as f32 * (1.0 - activity)).round() as u32;
            let g = (0xFF as f32 - (0xFF - 0xE5) as f32 * activity).round() as u32;
            let b = 0xFF_u32;
            let bg = (a << 24) | (r << 16) | (g << 8) | b;
            results.push((rect, bg));
        }
    } else {
        let mut total_w = 0.0_f32;
        let mut dot_widths = Vec::with_capacity(count);
        for p in 0..count {
            let dist = (page_float - p as f32).abs();
            let raw_activity = (1.0 - dist).clamp(0.0, 1.0);
            let activity = raw_activity * raw_activity * (3.0 - 2.0 * raw_activity);
            let w = (1.6 + 2.0 * activity) * vmin_px;
            dot_widths.push((w, activity));
            total_w += w;
        }
        total_w += (count.saturating_sub(1) as f32) * gap;

        let mut cur_x = container_rect.x + (container_rect.width - total_w) * 0.5;
        let center_y = container_rect.y + (container_rect.height - dot_base) * 0.5;
        for (w, activity) in dot_widths {
            let rect = crate::Rect::new(cur_x, center_y, w, dot_base);
            cur_x += w + gap;

            let a = (0x55 as f32 + (0xFF - 0x55) as f32 * activity).round() as u32;
            let r = (0xFF as f32 * (1.0 - activity)).round() as u32;
            let g = (0xFF as f32 - (0xFF - 0xE5) as f32 * activity).round() as u32;
            let b = 0xFF_u32;
            let bg = (a << 24) | (r << 16) | (g << 8) | b;
            results.push((rect, bg));
        }
    }
    results
}

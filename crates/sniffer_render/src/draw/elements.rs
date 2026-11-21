use super::draw_ui_node;
use sniffer_core::layout::LayoutNode;
use sniffer_core::render::Renderer;
use sniffer_core::types::Element;
use sniffer_core::ui::data_map::DataMap;
use sniffer_core::ui::style_map::StyleMap;
use sniffer_core::{Rect, ScreenMetrics};

const DEFAULT_TEXT_COLOR: u32 = 0xFF00_0000;

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn draw_element_contents(
    renderer: &mut dyn Renderer,
    element: &Element,
    layout: &LayoutNode,
    metrics: &ScreenMetrics,
    style_map: &StyleMap,
    data_map: &DataMap,
    transition_manager: &sniffer_core::anim::TransitionManager,
    final_alpha: f32,
    accumulated_scroll_x: f32,
    accumulated_scroll_y: f32,
    accumulated_tx: f32,
    accumulated_ty: f32,
    base_style: &sniffer_core::style::Style,
    rect: Rect,
    widget_id: Option<u64>,
    is_animating: bool,
    scroll_offsets: Option<&std::collections::HashMap<u64, (f32, f32)>>,
    page_indicator_float: Option<f32>,
) -> usize {
    let mut rendered_children = 0;
    match element {
        Element::Container { children, .. } | Element::SharedView { children, .. } => {
            for (child_el, child_lay) in children.iter().zip(layout.children.iter()) {
                rendered_children += draw_ui_node(
                    renderer,
                    child_el,
                    child_lay,
                    metrics,
                    style_map,
                    data_map,
                    transition_manager,
                    final_alpha,
                    accumulated_scroll_x,
                    accumulated_scroll_y,
                    accumulated_tx,
                    accumulated_ty,
                    is_animating,
                    scroll_offsets,
                    page_indicator_float,
                );
            }
        }
        Element::PageIndicator {
            page_count,
            orientation,
            ..
        } => {
            let count = page_count.unwrap_or(0) as usize;
            if count > 0 {
                let vmin_px = metrics.physical_width.min(metrics.physical_height) / 100.0;
                // Before the physics pipeline reports a page position the
                // indicator rests on the first page, matching the initial
                // state of a freshly mounted pager.
                let page_float = page_indicator_float.unwrap_or(0.0);
                let vertical =
                    *orientation == sniffer_core::types::PageIndicatorOrientation::Vertical;
                for (dot_rect, dot_bg) in sniffer_core::physics::indicator::compute_indicator_dots(
                    count, page_float, vmin_px, vertical, rect,
                ) {
                    let radius = (dot_rect.width.min(dot_rect.height)) * 0.5;
                    renderer.draw_rect(dot_rect, dot_bg, radius, 0.0, None);
                    rendered_children += 1;
                }
            }
        }
        Element::ScrollView {
            children,
            scroll_x,
            scroll_y,
            ..
        } => {
            renderer.push_clip_rect(rect, base_style.border_radius);

            let mut min_y = f32::MAX;
            let mut max_y = f32::MIN;
            for child_lay in layout.children.iter() {
                if child_lay.rect.y < min_y {
                    min_y = child_lay.rect.y;
                }
                let bottom = child_lay.rect.y + child_lay.rect.height;
                if bottom > max_y {
                    max_y = bottom;
                }
            }
            let content_height = if min_y <= max_y { max_y - min_y } else { 0.0 };

            let (raw_sx, raw_sy) = widget_id
                .and_then(|wid| scroll_offsets.and_then(|m| m.get(&wid).copied()))
                .unwrap_or((*scroll_x, *scroll_y));
            let safe_scroll_y = if raw_sy.is_nan() || raw_sy.is_infinite() {
                0.0
            } else {
                raw_sy
            };
            let safe_scroll_x = if raw_sx.is_nan() || raw_sx.is_infinite() {
                0.0
            } else {
                raw_sx
            };

            renderer.push_transform(0.0, 0.0, 1.0, 0.0, -safe_scroll_x, -safe_scroll_y);
            for (child_el, child_lay) in children.iter().zip(layout.children.iter()) {
                rendered_children += draw_ui_node(
                    renderer,
                    child_el,
                    child_lay,
                    metrics,
                    style_map,
                    data_map,
                    transition_manager,
                    final_alpha,
                    accumulated_scroll_x + safe_scroll_x,
                    accumulated_scroll_y + safe_scroll_y,
                    accumulated_tx,
                    accumulated_ty,
                    is_animating,
                    scroll_offsets,
                    page_indicator_float,
                );
            }
            renderer.pop_transform();
            renderer.pop_clip_rect();

            if content_height > rect.height {
                renderer.set_global_alpha(final_alpha * 0.5);
                let ratio = rect.height / content_height;
                let scrollbar_height = (rect.height * ratio).max(20.0);

                let max_scroll = content_height - rect.height;
                let scroll_pct = (safe_scroll_y / max_scroll).clamp(0.0, 1.0);

                let scrollbar_y = rect.y + (rect.height - scrollbar_height) * scroll_pct;
                let scrollbar_rect = Rect::new(
                    rect.x + rect.width - 6.0,
                    scrollbar_y,
                    4.0,
                    scrollbar_height,
                );

                renderer.draw_rect(scrollbar_rect, 0xFF00_0000, 2.0, 0.0, None);
                renderer.set_global_alpha(final_alpha);
            }
        }
        Element::Label { text, .. } => {
            let display_text = if let Some(w_id) = widget_id {
                data_map.label(w_id).unwrap_or_else(|| text.clone())
            } else {
                text.clone()
            };

            let color = base_style.text_color.unwrap_or(DEFAULT_TEXT_COLOR);
            let text_w = renderer.measure_text(&display_text, base_style.text_size);

            let draw_x = rect.x + (rect.width - text_w) / 2.0;
            let draw_y = rect.y + (rect.height - base_style.text_size) / 2.0;

            renderer.draw_text(&display_text, draw_x, draw_y, base_style.text_size, color);
        }
        Element::Image { src, id, .. } => {
            let img_id = if renderer.has_image(src.as_str()) {
                src.as_str()
            } else if let Some(elem_id) = id
                && renderer.has_image(elem_id.as_str())
            {
                elem_id.as_str()
            } else {
                src.as_str()
            };

            if !renderer.has_image(img_id) {
                // Custom-scheme images are delegated to whichever handler the
                // embedding host registered for that scheme; the engine
                // itself stays scheme-blind.
                super::image_schemes::request_scheme_image(src);
                return 0;
            }

            renderer.draw_image(
                img_id,
                rect,
                base_style.border_radius,
                base_style.object_fit,
            );
        }
        Element::TextInput {
            value,
            focused,
            placeholder,
            ..
        } => {
            renderer.push_clip_rect(rect, base_style.border_radius);

            let (display_text, color) = if *focused {
                (
                    format!("{value}_"),
                    base_style.text_color.unwrap_or(DEFAULT_TEXT_COLOR),
                )
            } else if value.is_empty() {
                if let Some(ph) = placeholder.as_ref().filter(|s| !s.is_empty()) {
                    (ph.clone(), 0x8894_A3B8)
                } else {
                    (
                        String::new(),
                        base_style.text_color.unwrap_or(DEFAULT_TEXT_COLOR),
                    )
                }
            } else {
                (
                    value.clone(),
                    base_style.text_color.unwrap_or(DEFAULT_TEXT_COLOR),
                )
            };

            let ts = base_style.text_size;
            let draw_y = rect.y + (rect.height - ts) / 2.0;
            let draw_x = rect.x + base_style.padding.left;

            if !display_text.is_empty() {
                renderer.draw_text(&display_text, draw_x, draw_y, ts, color);
            }

            renderer.pop_clip_rect();
        }
        Element::Checkbox { checked, .. } => {
            if *checked {
                let inner_rect = Rect::new(
                    rect.x + 4.0,
                    rect.y + 4.0,
                    rect.width - 8.0,
                    rect.height - 8.0,
                );
                renderer.draw_rect(
                    inner_rect,
                    base_style.text_color.unwrap_or(0xFF00_0000),
                    2.0,
                    0.0,
                    None,
                );
            }
        }
        Element::Slider {
            value, min, max, ..
        } => {
            let track_rect = Rect::new(rect.x, rect.y + rect.height / 2.0 - 2.0, rect.width, 4.0);
            renderer.draw_rect(track_rect, 0xFF88_8888, 2.0, 0.0, None);

            let range = (max - min).max(0.0001);
            let pct = ((value - min) / range).clamp(0.0, 1.0);
            let thumb_x = rect.x + (rect.width - 16.0) * pct;
            let thumb_rect = Rect::new(thumb_x, rect.y + rect.height / 2.0 - 8.0, 16.0, 16.0);
            renderer.draw_rect(
                thumb_rect,
                base_style.text_color.unwrap_or(0xFF00_0000),
                8.0,
                0.0,
                None,
            );
        }
        Element::ProgressBar { value, max, .. } => {
            let pct = (value / max.max(0.0001)).clamp(0.0, 1.0);
            let fill_rect = Rect::new(rect.x, rect.y, rect.width * pct, rect.height);
            renderer.draw_rect(
                fill_rect,
                base_style.text_color.unwrap_or(0xFF00_00FF),
                base_style.border_radius,
                0.0,
                None,
            );
        }
    }
    rendered_children
}

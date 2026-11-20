use sniffer_core::anim::TransitionManager;
use sniffer_core::layout::LayoutNode;
use sniffer_core::math::{Point, Rect, ScreenMetrics};
use sniffer_core::render::Renderer;
use sniffer_core::style::{ObjectFit, PointerEvents, Style};
use sniffer_core::types::{Element, PageIndicatorOrientation};
use sniffer_core::ui::{DataMap, StyleMap};
use sniffer_render::draw::{draw_ui_with_physics, find_clicked_button};

/// Records every filled rect so declarative pager indicator dot geometry
/// can be verified without a GPU backend.
#[derive(Default)]
struct DotRecordingRenderer {
    rects: Vec<(Rect, u32, f32)>,
}

impl Renderer for DotRecordingRenderer {
    fn clear(&mut self, _color: u32) {}
    fn draw_rect(
        &mut self,
        rect: Rect,
        color: u32,
        radius: f32,
        _border_width: f32,
        _border_color: Option<u32>,
    ) {
        self.rects.push((rect, color, radius));
    }
    fn draw_rect_gradient(
        &mut self,
        _rect: Rect,
        _color_top: u32,
        _color_bottom: u32,
        _radius: f32,
        _border_width: f32,
        _border_color: Option<u32>,
    ) {
    }
    fn draw_shadow(
        &mut self,
        _rect: Rect,
        _radius: f32,
        _offset_y: f32,
        _spread: f32,
        _color: u32,
    ) {
    }
    fn draw_circle(&mut self, _cx: f32, _cy: f32, _radius: f32, _color: u32) {}
    fn draw_text(&mut self, _text: &str, _x: f32, _y: f32, _size: f32, _color: u32) {}
    fn begin_frame(&mut self, _width: f32, _height: f32) {}
    fn end_frame(&mut self) {}
    fn set_clip_rect(&mut self, _rect: Rect) {}
    fn clear_clip_rect(&mut self) {}
    fn pop_clip_rect(&mut self) {}
    fn current_clip(&self) -> Option<&sniffer_core::render::ClipRegion> {
        None
    }
    fn set_global_alpha(&mut self, _alpha: f32) {}
    fn load_image(&mut self, _id: &str, _rgba_pixels: &[u8], _width: u32, _height: u32) {}
    fn has_image(&self, _id: &str) -> bool {
        false
    }
    fn draw_image(&mut self, _id: &str, _rect: Rect, _radius: f32, _object_fit: ObjectFit) {}
    fn measure_text(&self, _text: &str, _size: f32) -> f32 {
        0.0
    }
}

const ACTIVE_DOT: u32 = 0xFF00_E5FF;
const INACTIVE_DOT: u32 = 0x55FF_FFFF;

fn indicator_element(orientation: PageIndicatorOrientation, page_count: u32) -> Element {
    Element::PageIndicator {
        id: Some("grid_page_indicator".to_string()),
        page_count: Some(page_count),
        orientation,
        style: Style::default(),
    }
}

fn draw_indicator(
    element: &Element,
    rect: Rect,
    page_float: Option<f32>,
) -> (DotRecordingRenderer, usize) {
    let mut renderer = DotRecordingRenderer::default();
    let layout = LayoutNode::new(0, None, rect);
    let metrics = ScreenMetrics::from_dpi(1080.0, 2400.0, 160.0);
    let rendered = draw_ui_with_physics(
        &mut renderer,
        element,
        &layout,
        &metrics,
        &StyleMap::default(),
        &DataMap::default(),
        &TransitionManager::default(),
        1.0,
        0.0,
        0.0,
        None,
        page_float,
    );
    (renderer, rendered)
}

#[test]
fn page_indicator_draws_one_dot_per_page() {
    // vmin on a 1080x2400 surface is 10.8 px.
    let (renderer, rendered) = draw_indicator(
        &indicator_element(PageIndicatorOrientation::Horizontal, 4),
        Rect::new(0.0, 2300.0, 1080.0, 38.88),
        Some(1.0),
    );

    // `rendered` counts the indicator node itself plus one entry per dot.
    assert_eq!(rendered, 5);
    assert_eq!(renderer.rects.len(), 4);

    let dot_base = 1.6 * 10.8;
    let active_pill = 3.6 * 10.8;
    for (i, (rect, color, _radius)) in renderer.rects.iter().enumerate() {
        let is_active = i == 1;
        // The active page stretches into a pill and tints to accent.
        assert_eq!(*color, if is_active { ACTIVE_DOT } else { INACTIVE_DOT });
        let expected_w = if is_active { active_pill } else { dot_base };
        assert!((rect.width - expected_w).abs() < 0.01);
        assert!((rect.height - dot_base).abs() < 0.01);
    }

    // Dots must stay inside the indicator rect and advance horizontally.
    for (rect, _, _) in &renderer.rects {
        assert!(rect.x >= 0.0 && rect.x + rect.width <= 1080.0 + 0.01);
        assert!(rect.y >= 2300.0 && rect.y + rect.height <= 2338.89);
    }
    assert!(renderer.rects[1].0.x > renderer.rects[0].0.x);
    assert!(renderer.rects[2].0.x > renderer.rects[1].0.x);
}

#[test]
fn page_indicator_without_pager_data_rests_on_first_page() {
    let (renderer, rendered) = draw_indicator(
        &indicator_element(PageIndicatorOrientation::Horizontal, 3),
        Rect::new(0.0, 2300.0, 1080.0, 38.88),
        None,
    );

    assert_eq!(rendered, 4);
    assert_eq!(renderer.rects.len(), 3);
    assert_eq!(renderer.rects[0].1, ACTIVE_DOT);
    for (rect, color, _) in &renderer.rects[1..] {
        assert_eq!(*color, INACTIVE_DOT);
        assert!(rect.x > renderer.rects[0].0.x);
    }
}

#[test]
fn page_indicator_zero_page_count_draws_nothing() {
    let (renderer, rendered) = draw_indicator(
        &indicator_element(PageIndicatorOrientation::Vertical, 0),
        Rect::new(0.0, 0.0, 38.88, 2400.0),
        Some(2.0),
    );

    assert_eq!(rendered, 1);
    assert!(renderer.rects.is_empty());
}

#[test]
fn legacy_indicator_container_name_is_no_longer_special() {
    // A container that happens to carry the legacy magic ID now renders its
    // real children like any other container; no dots are synthesized.
    let legacy = Element::Container {
        id: Some("page_indicator_container".to_string()),
        style: Style::default(),
        children: vec![
            Element::Label {
                id: Some("dot_a".to_string()),
                text: "a".to_string(),
                style: Style::default(),
            },
            Element::Label {
                id: Some("dot_b".to_string()),
                text: "b".to_string(),
                style: Style::default(),
            },
        ],
    };

    let mut layout = LayoutNode::new(0, None, Rect::new(0.0, 2300.0, 1080.0, 38.88));
    layout.children = vec![
        LayoutNode::new(1, None, Rect::new(10.0, 2310.0, 8.0, 8.0)),
        LayoutNode::new(2, None, Rect::new(30.0, 2310.0, 8.0, 8.0)),
    ];

    let mut renderer = DotRecordingRenderer::default();
    let rendered = draw_ui_with_physics(
        &mut renderer,
        &legacy,
        &layout,
        &ScreenMetrics::from_dpi(1080.0, 2400.0, 160.0),
        &StyleMap::default(),
        &DataMap::default(),
        &TransitionManager::default(),
        1.0,
        0.0,
        0.0,
        None,
        Some(2.0),
    );

    // Container itself + two labels; critically, no synthesized dot rects.
    assert_eq!(rendered, 3);
    assert!(renderer.rects.is_empty());
}

#[test]
fn page_indicator_is_never_a_click_target() {
    let indicator_layout = LayoutNode::new(
        1,
        Some(sniffer_core::ui::widget::fnv1a(b"grid_page_indicator")),
        Rect::new(0.0, 2300.0, 1080.0, 38.88),
    );
    let root_layout = LayoutNode::new(0, None, Rect::new(0.0, 0.0, 1080.0, 2400.0));

    // Even with `pointer_events: auto`, the indicator is decorative by type.
    let root = Element::Container {
        id: Some("root".to_string()),
        style: Style {
            pointer_events: PointerEvents::None,
            ..Style::default()
        },
        children: vec![indicator_element(PageIndicatorOrientation::Horizontal, 4)],
    };
    let mut root_layout = root_layout;
    root_layout.children = vec![indicator_layout];

    // A tap inside the indicator area falls through to the root (which also
    // opts out), so no target is hit at all.
    let hit = find_clicked_button(&root, &root_layout, Point::new(500.0, 2320.0));
    assert_eq!(hit, None);
}

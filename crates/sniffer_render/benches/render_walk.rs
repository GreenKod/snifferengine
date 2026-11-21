//! 120 Hz frame-time regression benchmarks for the render phase.
//!
//! A no-op `Renderer` implementation measures the full CPU-side draw walk
//! (transform resolution, viewport culling, batch bookkeeping, hit-test
//! supporting geometry) over the same launcher-shaped 300+ element fixture
//! used by `sniffer_core/benches/frame_pipeline.rs`. GPU work is out of
//! scope by design: this isolates the engine-side cost that a 120 Hz frame
//! budget must absorb.

use criterion::{Criterion, criterion_group, criterion_main};
use sniffer_core::anim::TransitionManager;
use sniffer_core::layout::calculate_layout;
use sniffer_core::math::{Rect, ScreenMetrics, Size};
use sniffer_core::render::Renderer;
use sniffer_core::style::{Dimension, ObjectFit, Style};
use sniffer_core::types::Element;
use sniffer_core::ui::{DataMap, StyleMap};
use sniffer_render::draw::find_clicked_button_with_scroll;
use std::collections::HashMap;
use std::hint::black_box;

const SCREEN: Size = Size::new(1080.0, 2400.0);

/// A `Renderer` that records nothing: the draw walk runs to completion and
/// every dispatch lands in an empty sink, isolating traversal cost.
#[derive(Default)]
struct NullRenderer;

impl Renderer for NullRenderer {
    fn clear(&mut self, _color: u32) {}
    fn draw_rect(
        &mut self,
        _rect: Rect,
        _color: u32,
        _radius: f32,
        _border_width: f32,
        _border_color: Option<u32>,
    ) {
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

fn app_card(page: usize, index: usize) -> Element {
    Element::Container {
        id: Some(format!("card_{page}_{index}")),
        style: Style {
            width: Dimension::Percent(24.0),
            height: Dimension::Pixels(180.0),
            background_color: Some(0xFF20_4060),
            ..Style::default()
        },
        children: vec![
            Element::Label {
                id: Some(format!("card_{page}_{index}_title")),
                text: format!("Application {index}"),
                style: Style::default(),
            },
            Element::Label {
                id: Some(format!("card_{page}_{index}_subtitle")),
                text: "com.example.application".to_string(),
                style: Style::default(),
            },
        ],
    }
}

fn grid_page(page: usize) -> Element {
    Element::Container {
        id: Some(format!("grid_page_{page}")),
        style: Style {
            width: Dimension::Pixels(1080.0),
            height: Dimension::Pixels(1968.0),
            ..Style::default()
        },
        children: (0..4).map(|index| app_card(page, index)).collect(),
    }
}

/// Launcher-shaped fixture: six grid pages in a snap pager, a dock row, a
/// status bar label, and a page indicator — 300+ elements.
fn complex_tree() -> Element {
    let pages: Vec<Element> = (0..30).map(grid_page).collect();

    let pager = Element::ScrollView {
        id: Some("bench_grid_pager".to_string()),
        style: Style {
            width: Dimension::Pixels(1080.0),
            height: Dimension::Pixels(1968.0),
            overflow_hidden: true,
            ..Style::default()
        },
        children: vec![Element::Container {
            id: Some("bench_grid_track".to_string()),
            style: Style {
                flex_direction: sniffer_core::style::FlexDirection::Row,
                ..Style::default()
            },
            children: pages,
        }],
        scroll_x: 0.0,
        scroll_y: 0.0,
        scroll_sensitivity: None,
        dynamic_sensitivity: None,
        momentum_scrolling: None,
        capture_drag: None,
        snap_x: Some(1080.0),
        snap_y: None,
        rubber_band: None,
        page_count: Some(30),
        on_snap: None,
        dismiss_config: None,
    };

    let dock: Vec<Element> = (0..8)
        .map(|index| Element::Label {
            id: Some(format!("bench_dock_{index}")),
            text: format!("Dock {index}"),
            style: Style::default(),
        })
        .collect();

    Element::Container {
        id: Some("bench_root".to_string()),
        style: Style::default(),
        children: vec![
            pager,
            Element::Container {
                id: Some("bench_dock_row".to_string()),
                style: Style {
                    flex_direction: sniffer_core::style::FlexDirection::Row,
                    height: Dimension::Pixels(432.0),
                    ..Style::default()
                },
                children: dock,
            },
            Element::Label {
                id: Some("bench_status_bar".to_string()),
                text: "12:00 - Battery 100%".to_string(),
                style: Style::default(),
            },
            Element::PageIndicator {
                id: Some("bench_indicator".to_string()),
                page_count: Some(30),
                orientation: sniffer_core::types::PageIndicatorOrientation::Horizontal,
                style: Style::default(),
            },
        ],
    }
}

/// Runs the full draw walk over the laid-out fixture at the current page
/// offset, the per-frame render cost a 120 Hz budget must absorb.
fn bench_render_walk_full_tree(c: &mut Criterion) {
    let tree = complex_tree();
    let layout = calculate_layout(&tree, SCREEN, 0.0, 0.0);
    let mut renderer = NullRenderer;
    let metrics = ScreenMetrics::from_dpi(1080.0, 2400.0, 160.0);
    let transitions = TransitionManager::default();
    let mut scroll_offsets = HashMap::new();
    scroll_offsets.insert(
        sniffer_core::ui::widget::fnv1a(b"bench_grid_pager"),
        (0.0, 0.0),
    );

    c.bench_function("render_walk/full_tree", |b| {
        b.iter(|| {
            let drawn = sniffer_render::draw::draw_ui_with_physics(
                &mut renderer,
                black_box(&tree),
                black_box(&layout),
                black_box(&metrics),
                &StyleMap::default(),
                &DataMap::default(),
                black_box(&transitions),
                1.0,
                0.0,
                0.0,
                black_box(Some(&scroll_offsets)),
                Some(2.0),
            );
            black_box(drawn);
        });
    });
}

/// Resolves a tap at the center of the screen, the per-frame hit-test cost
/// walking the full element/layout pair.
fn bench_render_hit_test(c: &mut Criterion) {
    let tree = complex_tree();
    let layout = calculate_layout(&tree, SCREEN, 0.0, 0.0);
    let point = sniffer_core::math::Point::new(500.0, 900.0);
    let mut scroll_offsets = HashMap::new();
    scroll_offsets.insert(
        sniffer_core::ui::widget::fnv1a(b"bench_grid_pager"),
        (0.0, 0.0),
    );

    c.bench_function("render_walk/hit_test_full_tree", |b| {
        b.iter(|| {
            let hit = find_clicked_button_with_scroll(
                black_box(&tree),
                black_box(&layout),
                black_box(point),
                &|_id, sx, sy| (sx, sy),
            );
            black_box(hit);
            black_box(&scroll_offsets);
        });
    });
}

criterion_group!(
    render_walk,
    bench_render_walk_full_tree,
    bench_render_hit_test
);
criterion_main!(render_walk);

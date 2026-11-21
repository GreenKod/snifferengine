//! 120 Hz frame-time regression benchmarks for the layout and transform
//! phases of the frame pipeline.
//!
//! The fixture is a launcher-shaped tree with 200+ elements (paged app
//! grid with card lists, dock, status bar). `cargo bench` reports each
//! stage with microsecond precision; the baselines recorded under
//! `target/criterion` become the regression reference for the CI gates.
//!
//! Render-walk benchmarks live in `sniffer_render/benches/render_walk.rs`.

use criterion::{Criterion, criterion_group, criterion_main};
use sniffer_core::anim::TransitionManager;
use sniffer_core::layout::calculate_layout;
use sniffer_core::math::Size;
use sniffer_core::style::{Dimension, Easing, Style, Transition};
use sniffer_core::types::Element;
use sniffer_core::ui::widget::fnv1a;
use sniffer_core::virtualization::VirtualPageManager;
use std::hint::black_box;

const SCREEN: Size = Size::new(1080.0, 2400.0);

/// One launcher-style app card: container + title + subtitle.
fn app_card(page: usize, index: usize) -> Element {
    Element::Container {
        id: Some(format!("card_{page}_{index}")),
        style: Style {
            width: Dimension::Percent(24.0),
            height: Dimension::Pixels(180.0),
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

/// A pager page holding four cards (12 elements with labels).
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

/// A launcher-shaped tree: thirty grid pages inside a snap pager, a dock
/// row, and a page indicator — 400+ elements in total, with a pager long
/// enough (>= 24 flow children) to exercise visible-range culling.
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
            Element::PageIndicator {
                id: Some("bench_indicator".to_string()),
                page_count: Some(30),
                orientation: sniffer_core::types::PageIndicatorOrientation::Horizontal,
                style: Style::default(),
            },
        ],
    }
}

fn count_elements(element: &Element) -> usize {
    1 + match element {
        Element::Container { children, .. }
        | Element::ScrollView { children, .. }
        | Element::SharedView { children, .. } => {
            children.iter().map(count_elements).sum::<usize>()
        }
        _ => 0,
    }
}

/// Lays out the full 400+ element tree from scratch.
fn bench_layout_full(c: &mut Criterion) {
    let tree = complex_tree();
    assert!(
        count_elements(&tree) >= 200,
        "the regression fixture must stay above 200 elements"
    );
    c.bench_function("layout/full_tree", |b| {
        b.iter(|| {
            let layout = calculate_layout(black_box(&tree), black_box(SCREEN), 0.0, 0.0);
            black_box(layout.count_nodes());
        });
    });
}

/// Lays out the same tree after visible-range culling moved the window to
/// the last page: the reduced tree is what a frame pays after a fling.
fn bench_layout_culled_tree(c: &mut Criterion) {
    let mut tree = complex_tree();
    let base_layout = calculate_layout(&tree, SCREEN, 0.0, 0.0);
    let sv_id = fnv1a(b"bench_grid_pager");
    let mut manager = VirtualPageManager::new();
    assert!(
        manager.update_scroll_window(sv_id, (29.0 * 1080.0, 0.0), &mut tree, &base_layout),
        "the fixture pager must cull at the last page"
    );

    c.bench_function("layout/window_culled_tree", |b| {
        b.iter(|| {
            let layout = calculate_layout(black_box(&tree), black_box(SCREEN), 0.0, 0.0);
            black_box(layout.count_nodes());
        });
    });
}

/// Advances 200 concurrent spring transitions by one 120 Hz frame and
/// resolves every animated style. Targets are re-armed every iteration so
/// each sample measures a genuinely animating frame rather than the
/// permanently-settled steady state.
fn bench_transform_200_springs(c: &mut Criterion) {
    let mut manager = TransitionManager::default();
    let pressed = Style {
        elevation: 6.0,
        transform: sniffer_core::style::Transform {
            scale: 0.9,
            ..sniffer_core::style::Transform::default()
        },
        transition: Transition {
            duration: 0.24,
            easing: Easing::Spring {
                stiffness: 320.0,
                damping: 26.0,
            },
            ..Transition::default()
        },
        ..Style::default()
    };
    let ids: Vec<String> = (0..200)
        .map(|index| format!("bench_card_{index}"))
        .collect();

    let dt = 1.0 / 120.0;
    c.bench_function("transform/200_springs_tick_120hz", |b| {
        b.iter(|| {
            for id in &ids {
                manager.update_target(black_box(id), black_box(&pressed));
            }
            manager.tick(black_box(dt));
            let mut resolved = 0usize;
            for id in &ids {
                if manager.get_current_style(black_box(id)).is_some() {
                    resolved += 1;
                }
            }
            black_box(resolved);
        });
    });
}

criterion_group!(
    frame_pipeline,
    bench_layout_full,
    bench_layout_culled_tree,
    bench_transform_200_springs
);
criterion_main!(frame_pipeline);

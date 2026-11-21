use sniffer_core::types::Element;
use sniffer_core::virtualization::{ElementPool, POOL_MAX_CAPACITY, VirtualPageManager};

/// Builds a declarative pager with `pages` page containers inside a single
/// row container — the shape snap-scrolling requires. All IDs are neutral on
/// purpose: discovery must never depend on naming conventions.
fn pager_tree(pages: usize) -> Element {
    let mut page_children = Vec::with_capacity(pages);
    for p in 0..pages {
        page_children.push(Element::Container {
            id: Some(format!("feed_page_{p}")),
            style: sniffer_core::style::Style {
                width: Dimension::Pixels(360.0),
                height: Dimension::Pixels(640.0),
                ..sniffer_core::style::Style::default()
            },
            children: vec![Element::Label {
                id: Some(format!("feed_page_{p}_title")),
                text: format!("Page {p}"),
                style: sniffer_core::style::Style::default(),
            }],
        });
    }

    Element::ScrollView {
        id: Some("feed_pager".to_string()),
        // A pager declares its visible extent explicitly; an auto-sized
        // scroll container shrink-wraps its content and has no window to
        // cull against.
        style: sniffer_core::style::Style {
            width: Dimension::Pixels(360.0),
            height: Dimension::Pixels(640.0),
            overflow_hidden: true,
            ..sniffer_core::style::Style::default()
        },
        children: vec![Element::Container {
            id: Some("feed_row".to_string()),
            style: sniffer_core::style::Style {
                flex_direction: sniffer_core::style::FlexDirection::Row,
                ..sniffer_core::style::Style::default()
            },
            children: page_children,
        }],
        scroll_x: 0.0,
        scroll_y: 0.0,
        scroll_sensitivity: Some(1.0),
        dynamic_sensitivity: Some(true),
        momentum_scrolling: Some(true),
        capture_drag: Some(true),
        snap_x: Some(360.0),
        snap_y: None,
        rubber_band: None,
        page_count: Some(pages as u32),
        on_snap: None,
        dismiss_config: None,
    }
}

fn page_at(root: &mut Element, index: usize) -> &mut Element {
    let Element::ScrollView { children, .. } = root else {
        panic!("root must be a pager");
    };
    let Element::Container { children: row, .. } = &mut children[0] else {
        panic!("pager child must be the page row");
    };
    &mut row[index]
}

#[test]
fn discovers_pager_and_track_through_the_declarative_contract() {
    let mut root = pager_tree(3);
    let mut manager = VirtualPageManager::new();

    manager.strip_page(1, page_at(&mut root, 1));

    // The page slot is now an empty stub while its children live in cache.
    assert!(matches!(
        page_at(&mut root, 1),
        Element::Container { children, .. } if children.is_empty()
    ));
    manager.restore_page(1, page_at(&mut root, 1));
    assert!(matches!(
        page_at(&mut root, 1),
        Element::Container { children, .. } if children.len() == 1
    ));
}

#[test]
fn invalidate_on_resize_restores_every_cached_page() {
    let mut root = pager_tree(4);
    let mut manager = VirtualPageManager::new();

    manager.strip_page(0, page_at(&mut root, 0));
    manager.strip_page(2, page_at(&mut root, 2));
    manager.strip_page(3, page_at(&mut root, 3));

    manager.invalidate_on_resize(&mut root);

    for index in 0..4 {
        assert!(
            matches!(
                page_at(&mut root, index),
                Element::Container { children, .. } if children.len() == 1
            ),
            "page {index} must be fully materialized after resize invalidation"
        );
    }
}

#[test]
fn a_pager_without_page_count_is_not_a_virtualization_target() {
    // snap_x without page_count is a plain snap scroller, not a declarative
    // pager; discovery must skip it and resize invalidation must no-op.
    let mut root = pager_tree(2);
    if let Element::ScrollView { page_count, .. } = &mut root {
        *page_count = None;
    }

    let mut manager = VirtualPageManager::new();
    manager.strip_page(0, page_at(&mut root, 0));
    manager.invalidate_on_resize(&mut root);

    // The stripped children stay cached: no pager, no restoration target.
    assert!(matches!(
        page_at(&mut root, 0),
        Element::Container { children, .. } if children.is_empty()
    ));
}

#[test]
fn element_pool_recycles_up_to_capacity() {
    let mut pool = ElementPool::new(3);
    for i in 0..5 {
        pool.recycle(Element::Label {
            id: Some(format!("recycled_{i}")),
            text: String::new(),
            style: sniffer_core::style::Style::default(),
        });
    }

    assert_eq!(pool.available(), 3, "capacity 3 must cap the pool");
    assert!(pool.take().is_some());
    assert!(pool.take().is_some());
    assert!(pool.take().is_some());
    assert!(pool.take().is_none(), "the pool must be exhausted");

    let default_pool = ElementPool::default();
    assert_eq!(default_pool.available(), 0);
    assert_eq!(
        sniffer_core::virtualization::ElementPool::new(POOL_MAX_CAPACITY).available(),
        0
    );
}

#[test]
fn virtualize_tree_keeps_the_dormant_window_policy() {
    // The page-window policy is currently gated off (see
    // `virtualize_tree_with_velocity`): the tree must stay fully
    // materialized so mid-swipe relayout spikes cannot regress.
    let mut root = pager_tree(3);
    let mut manager = VirtualPageManager::new();

    manager.set_active_page(2, &mut root);
    manager.virtualize_tree_with_velocity(&mut root, 1200.0);
    manager.update_predicted_page(1, &mut root, -900.0);
    manager.on_page_changed(0, &mut root);

    for index in 0..3 {
        assert!(
            matches!(
                page_at(&mut root, index),
                Element::Container { children, .. } if children.len() == 1
            ),
            "page {index} must remain materialized while the window is dormant"
        );
    }
}

#[test]
fn find_element_by_id_mut_still_serves_generic_lookups() {
    let mut root = pager_tree(2);
    let found = VirtualPageManager::find_element_by_id_mut(&mut root, "feed_page_1_title");
    assert!(found.is_some());
    let missing = VirtualPageManager::find_element_by_id_mut(&mut root, "no_such_element");
    assert!(missing.is_none());
}

// =============================================================================
// Stage 6.2 — axis-aligned visible-range culling of ScrollView content
// =============================================================================

use sniffer_core::layout::calculate_layout;
use sniffer_core::math::Size;
use sniffer_core::physics::{ScrollPhysics, sync_max_scroll_from_layout};
use sniffer_core::style::{Dimension, PointerEvents, Style};
use sniffer_core::ui::widget::fnv1a;
use std::collections::HashMap;

const LIST_ID: &str = "feed_list";

fn vertical_list(count: usize) -> Element {
    let items: Vec<Element> = (0..count)
        .map(|i| Element::Container {
            id: Some(format!("row_{i}")),
            style: Style {
                height: Dimension::Pixels(100.0),
                ..Style::default()
            },
            children: vec![Element::Label {
                id: Some(format!("row_{i}_label")),
                text: format!("Row {i}"),
                style: Style::default(),
            }],
        })
        .collect();

    Element::ScrollView {
        id: Some(LIST_ID.to_string()),
        style: Style {
            width: Dimension::Pixels(400.0),
            height: Dimension::Pixels(800.0),
            ..Style::default()
        },
        children: items,
        scroll_x: 0.0,
        scroll_y: 0.0,
        scroll_sensitivity: None,
        dynamic_sensitivity: None,
        momentum_scrolling: None,
        capture_drag: None,
        snap_x: None,
        snap_y: None,
        rubber_band: None,
        page_count: None,
        on_snap: None,
        dismiss_config: None,
    }
}

fn list_children(root: &mut Element) -> &mut Vec<Element> {
    let Element::ScrollView { children, .. } = root else {
        panic!("root must be the list scroll view");
    };
    children
}

fn is_stub(element: &Element) -> bool {
    matches!(
        element,
        Element::Container { children, style, .. }
            if children.is_empty() && style.pointer_events == PointerEvents::None
    )
}

fn has_label(element: &Element) -> bool {
    matches!(element, Element::Container { children, .. } if !children.is_empty())
}

#[test]
fn vertical_scroll_content_is_window_culled() {
    let mut root = vertical_list(40);
    let layout = calculate_layout(&root, Size::new(400.0, 800.0), 0.0, 0.0);
    let sv_id = fnv1a(LIST_ID.as_bytes());
    let mut manager = VirtualPageManager::new();

    // Scrolled 2000px down a 4000px list: only rows overlapping
    // [1400, 3400] (viewport 800 + 0.75-viewport margins) stay materialized,
    // which is exactly the 22-row window [13, 35).
    let changed = manager.update_scroll_window(sv_id, (0.0, 2000.0), &mut root, &layout);
    assert!(changed, "the first window computation must cull");

    let stubs = list_children(&mut root)
        .iter()
        .filter(|c| is_stub(c))
        .count();
    assert_eq!(stubs, 18, "rows outside [13, 35) must be stubs");

    for (idx, child) in list_children(&mut root).iter().enumerate() {
        let start = idx as f32 * 100.0;
        let in_window = start + 100.0 >= 1400.0 && start <= 3400.0;
        assert_eq!(
            has_label(child),
            in_window,
            "row {idx} materialization must match its window membership"
        );
    }
}

#[test]
fn window_crossing_restores_stripped_children() {
    let mut root = vertical_list(40);
    let layout = calculate_layout(&root, Size::new(400.0, 800.0), 0.0, 0.0);
    let sv_id = fnv1a(LIST_ID.as_bytes());
    let mut manager = VirtualPageManager::new();

    assert!(manager.update_scroll_window(sv_id, (0.0, 2000.0), &mut root, &layout));
    assert!(
        is_stub(&list_children(&mut root)[0]),
        "row 0 starts stubbed"
    );
    assert!(
        has_label(&list_children(&mut root)[20]),
        "row 20 starts live"
    );

    // Fling back to the top: the top rows must come back, the bottom must
    // turn into stubs, and the child count must never change.
    assert!(manager.update_scroll_window(sv_id, (0.0, 0.0), &mut root, &layout));
    let children = list_children(&mut root);
    assert_eq!(children.len(), 40);
    assert!(has_label(&children[0]), "row 0 must be restored");
    assert!(is_stub(&children[39]), "row 39 must be stubbed far away");

    // A no-op update (same window) must report no change.
    assert!(!manager.update_scroll_window(sv_id, (0.0, 0.0), &mut root, &layout));
}

#[test]
fn culled_layout_preserves_scroll_extent() {
    // THE physics invariant: stub footprints keep the scrollable content
    // extent exact, so clamping cannot drift after culling or relayout.
    let mut root = vertical_list(40);
    let layout = calculate_layout(&root, Size::new(400.0, 800.0), 0.0, 0.0);
    let sv_id = fnv1a(LIST_ID.as_bytes());

    let mut full_physics: HashMap<u64, ScrollPhysics> = HashMap::new();
    full_physics.insert(sv_id, ScrollPhysics::default());
    sync_max_scroll_from_layout(&root, &layout, &mut full_physics);
    let full_max = full_physics[&sv_id].max_y;

    let mut manager = VirtualPageManager::new();
    assert!(manager.update_scroll_window(sv_id, (0.0, 2000.0), &mut root, &layout));

    // Relayout the CULLED tree: the reduced layout must produce the same
    // scroll extent as the fully materialized one.
    let culled_layout = calculate_layout(&root, Size::new(400.0, 800.0), 0.0, 0.0);
    let mut culled_physics: HashMap<u64, ScrollPhysics> = HashMap::new();
    culled_physics.insert(sv_id, ScrollPhysics::default());
    sync_max_scroll_from_layout(&root, &culled_layout, &mut culled_physics);
    let culled_max = culled_physics[&sv_id].max_y;

    assert_eq!(
        full_max,
        Some(3200.0),
        "40 rows of 100px in an 800px viewport"
    );
    assert_eq!(
        full_max, culled_max,
        "culling must not change the scrollable extent"
    );
}

#[test]
fn short_scroll_lists_are_never_culled() {
    let mut root = vertical_list(10);
    let layout = calculate_layout(&root, Size::new(400.0, 800.0), 0.0, 0.0);
    let sv_id = fnv1a(LIST_ID.as_bytes());
    let mut manager = VirtualPageManager::new();

    assert!(!manager.update_scroll_window(sv_id, (0.0, 0.0), &mut root, &layout));
    assert!(list_children(&mut root).iter().all(has_label));
}

#[test]
fn horizontal_snap_pagers_cull_along_the_page_axis() {
    let mut root = pager_tree(30);
    let layout = calculate_layout(&root, Size::new(360.0, 640.0), 0.0, 0.0);
    let sv_id = fnv1a(b"feed_pager");
    let mut manager = VirtualPageManager::new();

    // The pager's page row is the single wrapper container; culling follows
    // the horizontal axis declared by `snap_x`.
    let changed = manager.update_scroll_window(sv_id, (8000.0, 0.0), &mut root, &layout);
    assert!(changed, "a 30-page pager far from the origin must cull");

    let Element::ScrollView { children, .. } = &mut root else {
        panic!("root must be the pager");
    };
    let Element::Container { children: row, .. } = &mut children[0] else {
        panic!("pager child must be the page row");
    };
    assert_eq!(row.len(), 30, "page slots never change count");
    assert!(is_stub(&row[0]), "far-left pages must be stubs");
    assert!(has_label(&row[22]), "pages around the position stay live");
    assert!(is_stub(&row[29]), "far-right pages must be stubs");
}

#[test]
fn invalidate_on_resize_restores_the_full_list() {
    let mut root = vertical_list(40);
    let layout = calculate_layout(&root, Size::new(400.0, 800.0), 0.0, 0.0);
    let sv_id = fnv1a(LIST_ID.as_bytes());
    let mut manager = VirtualPageManager::new();

    assert!(manager.update_scroll_window(sv_id, (0.0, 2000.0), &mut root, &layout));
    assert!(list_children(&mut root).iter().any(is_stub));

    manager.invalidate_on_resize(&mut root);
    assert!(
        list_children(&mut root).iter().all(has_label),
        "resize invalidation must fully rematerialize the list"
    );
}

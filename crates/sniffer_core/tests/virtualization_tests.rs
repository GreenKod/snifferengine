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
            style: sniffer_core::style::Style::default(),
            children: vec![Element::Label {
                id: Some(format!("feed_page_{p}_title")),
                text: format!("Page {p}"),
                style: sniffer_core::style::Style::default(),
            }],
        });
    }

    Element::ScrollView {
        id: Some("feed_pager".to_string()),
        style: sniffer_core::style::Style::default(),
        children: vec![Element::Container {
            id: Some("feed_row".to_string()),
            style: sniffer_core::style::Style::default(),
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

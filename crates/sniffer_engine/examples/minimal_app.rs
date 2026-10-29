//! Minimal example demonstrating how to construct a declarative UI tree with
//! elastic `ScrollView` and `ScrollDismissConfig` using `sniffer_engine::prelude::*`.
//!
//! To run the full desktop window loop in a host project, call `sniffer_engine::run_desktop()`.

use sniffer_engine::prelude::*;
use std::collections::HashMap;

fn main() {
    let ui = Element::Container {
        id: Some("example_root".to_string()),
        style: Style::default(),
        children: vec![Element::ScrollView {
            id: Some("example_sheet_scroll".to_string()),
            style: Style::default(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            scroll_sensitivity: None,
            dynamic_sensitivity: None,
            momentum_scrolling: None,
            capture_drag: None,
            snap_x: None,
            snap_y: None,
            rubber_band: Some(0.55),
            page_count: None,
            on_snap: None,
            dismiss_config: Some(ScrollDismissConfig {
                direction: DismissDirection::Down,
                threshold: Some(60.0),
                target_container_id: Some("example_root".to_string()),
                action_event: Some("example_dismiss".to_string()),
            }),
            children: vec![Element::Label {
                id: Some("hello_label".to_string()),
                text: "Hello from SnifferEngine!".to_string(),
                style: Style::default(),
            }],
        }],
    };

    let layout = calculate_layout(&ui, Size::new(1080.0, 2400.0), 0.0, 0.0);
    let mut transition_manager = TransitionManager::default();
    transition_manager.sync_tree(&ui);

    let mut scroll_physics: HashMap<u64, ScrollPhysics> = HashMap::new();
    sync_scroll_physics_from_tree(&ui, &mut scroll_physics);
    sync_max_scroll_from_layout(&ui, &layout, &mut scroll_physics);

    println!(
        "SnifferEngine minimal example initialized: {} scroll container(s) synced.",
        scroll_physics.len()
    );
}

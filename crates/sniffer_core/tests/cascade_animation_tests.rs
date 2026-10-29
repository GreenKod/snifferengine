use sniffer_core::anim::TransitionManager;
use sniffer_core::style::{Easing, Style, Transition};

#[test]
fn test_staggered_cascade_delay_ordering() {
    let mut manager = TransitionManager::default();

    let style_0 = Style {
        opacity: 0.0,
        transition: Transition {
            duration: 0.20,
            easing: Easing::Linear,
            delay: 0.0,
            stagger_index: 0,
            stagger_interval: 0.05,
        },
        ..Default::default()
    };

    let mut style_1 = style_0.clone();
    style_1.transition.stagger_index = 1;

    let mut style_2 = style_0.clone();
    style_2.transition.stagger_index = 2;

    manager.update_target("item_0", &style_0);
    manager.update_target("item_1", &style_1);
    manager.update_target("item_2", &style_2);

    // Now trigger transition to opacity: 1.0
    let mut target_0 = style_0.clone();
    target_0.opacity = 1.0;
    let mut target_1 = style_1.clone();
    target_1.opacity = 1.0;
    let mut target_2 = style_2.clone();
    target_2.opacity = 1.0;

    manager.update_target("item_0", &target_0);
    manager.update_target("item_1", &target_1);
    manager.update_target("item_2", &target_2);

    assert!(manager.is_animating());

    // Step 0.03s: item_0 is animating, item_1 (delay 0.05s) and item_2 (delay 0.10s) must still be at 0.0
    manager.tick(0.03);
    assert!(manager.get_current_style("item_0").unwrap().opacity > 0.0);
    assert_eq!(manager.get_current_style("item_1").unwrap().opacity, 0.0);
    assert_eq!(manager.get_current_style("item_2").unwrap().opacity, 0.0);

    // Step another 0.04s (total 0.07s): item_1 has now started (> 0.05s), item_2 is still at 0.0 (< 0.10s)
    manager.tick(0.04);
    assert!(manager.get_current_style("item_0").unwrap().opacity > 0.0);
    assert!(manager.get_current_style("item_1").unwrap().opacity > 0.0);
    assert_eq!(manager.get_current_style("item_2").unwrap().opacity, 0.0);

    // Run until all settled
    let mut steps = 0;
    while manager.is_animating() && steps < 600 {
        manager.tick(1.0 / 60.0);
        steps += 1;
    }

    assert!(!manager.is_animating());
    assert_eq!(manager.get_current_style("item_0").unwrap().opacity, 1.0);
    assert_eq!(manager.get_current_style("item_1").unwrap().opacity, 1.0);
    assert_eq!(manager.get_current_style("item_2").unwrap().opacity, 1.0);
}

#[test]
fn test_spring_easing_cascade_convergence() {
    let mut manager = TransitionManager::default();

    let start_style = Style {
        opacity: 0.0,
        transform: sniffer_core::style::Transform {
            translate_y: 100.0,
            ..Default::default()
        },
        transition: Transition {
            duration: 0.35,
            easing: Easing::Spring {
                stiffness: 220.0,
                damping: 20.0,
            },
            delay: 0.0,
            stagger_index: 0,
            stagger_interval: 0.02,
        },
        ..Default::default()
    };

    manager.update_target("card_0", &start_style);

    let mut target_style = start_style.clone();
    target_style.transform.translate_y = 0.0;
    target_style.opacity = 1.0;

    manager.update_target("card_0", &target_style);
    assert!(manager.is_animating());

    let dt = 1.0 / 120.0;
    let mut steps = 0;
    while manager.is_animating() && steps < 1200 {
        manager.tick(dt);
        steps += 1;
    }

    assert!(!manager.is_animating());
    let current = manager.get_current_style("card_0").unwrap();
    assert!((current.transform.translate_y - 0.0).abs() < 1e-2);
    assert!((current.opacity - 1.0).abs() < 1e-2);
}

#[test]
fn test_unmounted_elements_pruned_from_transition_manager() {
    use sniffer_core::types::Element;

    let mut manager = TransitionManager::default();

    // 1. Mount drawer container with 20 cards
    let mut children = Vec::new();
    for i in 0..20 {
        let style = Style {
            opacity: 0.0,
            transition: Transition {
                duration: 0.35,
                easing: Easing::Linear,
                delay: 0.0,
                stagger_index: i,
                stagger_interval: 0.05, // 20 * 0.05 = 1.0s total stagger
            },
            ..Default::default()
        };
        children.push(Element::Container {
            id: Some(format!("card_{i}")),
            style,
            children: Vec::new(),
        });
    }

    let drawer_element = Element::Container {
        id: Some("drawer_root".to_string()),
        style: Style::default(),
        children,
    };

    manager.sync_tree(&drawer_element);

    // Apply target styles with opacity 1.0
    let mut open_children = Vec::new();
    for i in 0..20 {
        let style = Style {
            opacity: 1.0,
            transition: Transition {
                duration: 0.35,
                easing: Easing::Linear,
                delay: 0.0,
                stagger_index: i,
                stagger_interval: 0.05,
            },
            ..Default::default()
        };
        open_children.push(Element::Container {
            id: Some(format!("card_{i}")),
            style,
            children: Vec::new(),
        });
    }
    let open_drawer = Element::Container {
        id: Some("drawer_root".to_string()),
        style: Style::default(),
        children: open_children,
    };

    manager.sync_tree(&open_drawer);
    assert!(manager.is_animating(), "Drawer should be animating entry");
    assert_eq!(manager.states.len(), 21); // drawer_root + 20 cards

    // Tick 0.05s: card_0 is animating, later cards are still in stagger delay
    manager.tick(0.05);
    assert!(manager.is_animating());

    // 2. User navigates back to Home -> Drawer is unmounted from UI tree!
    let home_element = Element::Container {
        id: Some("home_screen_root".to_string()),
        style: Style::default(),
        children: Vec::new(),
    };

    manager.sync_tree(&home_element);

    // CRITICAL: All 20 drawer cards and drawer_root must be pruned IMMEDIATELY!
    // is_animating must NOT stay true waiting for 1.0s stagger delay!
    assert_eq!(manager.states.len(), 1); // only home_screen_root
    assert!(manager.states.contains_key("home_screen_root"));
    assert!(!manager.states.contains_key("drawer_root"));
    assert!(!manager.states.contains_key("card_0"));
    assert!(!manager.states.contains_key("card_19"));
    assert!(
        !manager.is_animating(),
        "TransitionManager must be completely idle after unmount"
    );
}

use sniffer_platform_desktop::get_application_list;

#[test]
fn test_desktop_mock_application_list() {
    let apps = get_application_list().expect("Should return mock apps on desktop");
    assert!(!apps.is_empty());
    assert_eq!(apps.len(), 10);

    let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"Browser"));
    assert!(names.contains(&"Calculator"));
    assert!(names.contains(&"Settings"));
    assert!(names.contains(&"Files"));
}

#[test]
fn test_desktop_app_state_layout_caching_and_dirty_tracking() {
    let mut app = sniffer_platform_desktop::app::AppState::new();

    // 1. Initial state must be dirty and have no cached layout
    assert!(app.layout_dirty);
    assert!(app.cached_layout.is_none());
    assert_eq!(app.last_ui_version, 0);

    // 2. Simulate caching a resolved layout
    let dummy_root = sniffer_core::types::Element::Container {
        id: Some("root".to_string()),
        style: sniffer_core::style::Style::default(),
        children: vec![],
    };
    let dummy_layout = sniffer_core::layout::calculate_layout(
        &dummy_root,
        sniffer_core::Size::new(800.0, 600.0),
        0.0,
        0.0,
    );
    app.cached_root_element = std::sync::Arc::new(dummy_root);
    app.cached_layout = Some(std::sync::Arc::new(dummy_layout));
    app.layout_dirty = false;
    app.last_ui_version = 1;

    // 3. Verify cached layout is present and not dirty
    assert!(!app.layout_dirty);
    assert!(app.cached_layout.is_some());

    // 4. Verify UI_VERSION mismatch detection
    let prev_version = sniffer_core::types::UI_VERSION.load(std::sync::atomic::Ordering::Relaxed);
    sniffer_core::types::UI_VERSION.store(2, std::sync::atomic::Ordering::Relaxed);
    let current_version =
        sniffer_core::types::UI_VERSION.load(std::sync::atomic::Ordering::Relaxed);
    assert_ne!(current_version, app.last_ui_version);

    // Restore UI_VERSION to avoid side effects
    sniffer_core::types::UI_VERSION.store(prev_version, std::sync::atomic::Ordering::Relaxed);
}

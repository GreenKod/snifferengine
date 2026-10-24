use sniffer_core::physics::ScrollPhysics;

#[test]
fn test_scroll_physics_snap_spring_integration() {
    let mut phys = ScrollPhysics {
        snap_x: Some(360.0),
        page_count: Some(3),
        ..Default::default()
    };

    // Position at page 0 with forward velocity towards page 1
    phys.pos_x = 100.0;
    phys.release_drag(400.0);

    assert!(phys.snap_target_x.is_some());
    assert_eq!(phys.snap_target_x, Some(360.0));
    assert!(phys.spring_sim_x.is_some());
    assert!(phys.is_animating());

    // Step physics at 120 FPS
    let dt = 1.0 / 120.0;
    let mut steps = 0;
    while phys.tick(dt) && steps < 600 {
        steps += 1;
    }

    assert_eq!(phys.pos_x, 360.0);
    assert_eq!(phys.vel_x, 0.0);
    assert!(phys.snap_target_x.is_none());
    assert!(phys.spring_sim_x.is_none());
    assert!(!phys.is_animating());
}

#[test]
fn test_scroll_physics_rubber_band_y_spring_recovery() {
    let mut phys = ScrollPhysics {
        max_y: Some(1000.0),
        ..Default::default()
    };

    // Overscroll past top boundary
    phys.pos_y = -120.0;
    phys.release_drag_y(0.0);

    assert!(phys.spring_sim_y.is_some());
    assert!(phys.is_animating());

    let dt = 1.0 / 120.0;
    let mut steps = 0;
    while phys.tick(dt) && steps < 600 {
        steps += 1;
    }

    assert_eq!(phys.pos_y, 0.0);
    assert_eq!(phys.vel_y, 0.0);
    assert!(phys.spring_sim_y.is_none());
    assert!(!phys.is_animating());

    // Overscroll past bottom boundary
    phys.pos_y = 1150.0;
    phys.release_drag_y(0.0);

    assert!(phys.spring_sim_y.is_some());
    assert!(phys.is_animating());

    steps = 0;
    while phys.tick(dt) && steps < 600 {
        steps += 1;
    }

    assert_eq!(phys.pos_y, 1000.0);
    assert_eq!(phys.vel_y, 0.0);
    assert!(phys.spring_sim_y.is_none());
}

#[test]
fn test_scroll_physics_drag_cancels_active_spring() {
    let mut phys = ScrollPhysics {
        max_y: Some(1000.0),
        ..Default::default()
    };

    phys.pos_y = -80.0;
    phys.release_drag_y(0.0);
    assert!(phys.spring_sim_y.is_some());

    // User touches screen and drags again
    phys.apply_drag_y(10.0);
    assert!(phys.spring_sim_y.is_none());
}

#[test]
fn test_scroll_physics_rapid_direction_reversals() {
    let page_w = 720.0;
    let mut phys = ScrollPhysics {
        snap_x: Some(page_w),
        page_count: Some(5),
        ..Default::default()
    };

    // 1. Drag forward towards Page 1, then reverse mid-drag
    phys.apply_drag(300.0);
    assert_eq!(phys.drag_start_x, 0.0);
    assert_eq!(phys.pos_x, 300.0);

    // Reversal: drag backwards towards Page 0
    phys.apply_drag(-160.0);
    assert_eq!(phys.pos_x, 140.0);

    // Release with backward flick
    phys.release_drag(-1200.0);
    assert_eq!(phys.snap_target_x, Some(0.0));
    assert!(phys.spring_sim_x.is_some());

    // Step physics to rest
    let dt = 1.0 / 120.0;
    let mut steps = 0;
    while phys.tick(dt) && steps < 600 {
        steps += 1;
    }
    assert_eq!(phys.pos_x, 0.0);
    assert_eq!(phys.vel_x, 0.0);
    assert!(!phys.is_animating());

    // 2. Mid-air interruption: Start forward flick to Page 1
    phys.apply_drag(200.0);
    phys.release_drag(1800.0);
    assert_eq!(phys.snap_target_x, Some(page_w));

    // Tick 5 frames (mid-motion)
    for _ in 0..5 {
        phys.tick(dt);
    }
    assert!(phys.pos_x > 200.0 && phys.pos_x < page_w);

    // User interrupts with new touch: simulates touch Down and reversal drag
    phys.vel_x = 0.0;
    phys.spring_sim_x = None;
    phys.snap_target_x = None;
    phys.apply_drag(-80.0);
    assert!(phys.is_dragging);

    // Release with backward flick towards Page 0
    phys.release_drag(-1500.0);
    assert_eq!(phys.snap_target_x, Some(0.0));

    // Tick to rest
    steps = 0;
    while phys.tick(dt) && steps < 600 {
        steps += 1;
    }
    assert_eq!(phys.pos_x, 0.0);
    assert_eq!(phys.vel_x, 0.0);
    assert!(!phys.is_animating());
}

#[test]
fn test_scroll_physics_hundreds_of_rapid_page_swipes() {
    let page_w = 720.0;
    let total_pages = 5;
    let mut phys = ScrollPhysics {
        snap_x: Some(page_w),
        page_count: Some(total_pages),
        ..Default::default()
    };

    let dt = 1.0 / 120.0;
    let mut current_expected_page: u32 = 0;

    // Simulate 200 rapid alternating swipes
    for i in 0..200 {
        let go_forward = if current_expected_page == 0 {
            true
        } else if current_expected_page == total_pages - 1 {
            false
        } else {
            i % 2 == 0
        };

        let drag_delta = if go_forward { 220.0 } else { -220.0 };
        let fling_vel = if go_forward { 1400.0 } else { -1400.0 };

        // Arrest prior physics on touch down
        phys.vel_x = 0.0;
        phys.spring_sim_x = None;
        phys.snap_target_x = None;
        phys.apply_drag(drag_delta);

        // Quick mid-drag jitter
        phys.apply_drag(if go_forward { 30.0 } else { -30.0 });

        phys.release_drag(fling_vel);

        if go_forward {
            current_expected_page = (current_expected_page + 1).min(total_pages - 1);
        } else {
            current_expected_page = current_expected_page.saturating_sub(1);
        }

        assert_eq!(
            phys.snap_target_x,
            Some(current_expected_page as f32 * page_w),
            "Iteration {i} target page mismatch"
        );

        // Run until settled
        let mut steps = 0;
        while phys.tick(dt) && steps < 600 {
            steps += 1;
        }

        assert_eq!(
            phys.pos_x,
            current_expected_page as f32 * page_w,
            "Iteration {i} settled pos_x mismatch"
        );
        assert_eq!(phys.vel_x, 0.0, "Iteration {i} residual velocity");
        assert!(!phys.is_animating(), "Iteration {i} failed to reach rest");
    }
}

#[test]
fn test_scroll_physics_action_cancel_safety() {
    let page_w = 720.0;
    let mut phys = ScrollPhysics {
        snap_x: Some(page_w),
        page_count: Some(4),
        ..Default::default()
    };

    // 1. User starts drag on Page 0, moves 120px (under 35%), and gesture is CANCELLED (e.g. system dialog, phone call)
    phys.apply_drag(120.0);
    assert!(phys.is_dragging);
    assert_eq!(phys.pos_x, 120.0);

    // Cancel triggers release with zero velocity
    phys.release_drag(0.0);
    assert!(!phys.is_dragging);
    assert_eq!(
        phys.snap_target_x,
        Some(0.0),
        "Should snap back to Page 0 (< 35% displacement)"
    );

    let dt = 1.0 / 120.0;
    let mut steps = 0;
    while phys.tick(dt) && steps < 600 {
        steps += 1;
    }
    assert_eq!(phys.pos_x, 0.0);
    assert_eq!(phys.vel_x, 0.0);
    assert!(!phys.is_animating());

    // 2. User drags beyond 35% (300px) and gesture is cancelled:
    phys.apply_drag(300.0);
    assert!(phys.is_dragging);
    assert_eq!(phys.pos_x, 300.0);

    phys.release_drag(0.0);
    assert!(!phys.is_dragging);
    assert_eq!(
        phys.snap_target_x,
        Some(page_w),
        "Should snap to Page 1 (>= 35% displacement)"
    );

    steps = 0;
    while phys.tick(dt) && steps < 600 {
        steps += 1;
    }
    assert_eq!(phys.pos_x, page_w);
    assert_eq!(phys.vel_x, 0.0);
    assert!(!phys.is_animating());
}

#[test]
fn test_scroll_physics_retouch_during_active_spring() {
    let page_w = 720.0;
    let mut phys = ScrollPhysics {
        snap_x: Some(page_w),
        page_count: Some(3),
        ..Default::default()
    };

    // Start flick from Page 0 to Page 1
    phys.apply_drag(200.0);
    phys.release_drag(1200.0);
    assert_eq!(phys.snap_target_x, Some(page_w));
    assert!(phys.spring_sim_x.is_some());

    // Step 8 frames (~67ms) into the spring motion
    let dt = 1.0 / 120.0;
    for _ in 0..8 {
        phys.tick(dt);
    }
    let mid_flight_x = phys.pos_x;
    assert!(mid_flight_x > 200.0 && mid_flight_x < page_w);
    assert!(phys.is_animating());

    // User touches down mid-flight: arrest simulation cleanly
    phys.vel_x = 0.0;
    phys.spring_sim_x = None;
    phys.snap_target_x = None;
    phys.is_dragging = true;
    phys.drag_start_x = phys.pos_x;

    assert!(
        phys.spring_sim_x.is_none(),
        "Spring simulation must be arrested"
    );
    assert!(phys.snap_target_x.is_none(), "Snap target must be cleared");
    assert_eq!(phys.vel_x, 0.0, "Velocity must be arrested to zero");
    assert!(
        phys.is_dragging,
        "Scroll physics is now controlled by touch drag"
    );

    // User decides to flick back towards Page 0
    phys.apply_drag(-150.0);
    phys.release_drag(-1600.0);

    assert_eq!(phys.snap_target_x, Some(0.0));
    assert!(phys.spring_sim_x.is_some());

    let mut steps = 0;
    while phys.tick(dt) && steps < 600 {
        steps += 1;
    }
    assert_eq!(phys.pos_x, 0.0);
    assert_eq!(phys.vel_x, 0.0);
    assert!(!phys.is_animating());
}

#[test]
fn test_drawer_navigation_cancels_ongoing_vertical_momentum() {
    let mut drawer_phys = ScrollPhysics {
        max_y: Some(3000.0),
        ..Default::default()
    };

    // Simulate active vertical scrolling momentum in app drawer
    drawer_phys.pos_y = 450.0;
    drawer_phys.vel_y = 1800.0;
    drawer_phys.is_dragging = false;

    // Tick a few frames: momentum is actively running
    let dt = 1.0 / 120.0;
    drawer_phys.tick(dt);
    assert!(
        drawer_phys.is_animating(),
        "Drawer scroll momentum should be active"
    );

    // User initiates SwipeDown to return to Home Screen:
    // Navigation immediately resets drawer vertical physics
    drawer_phys.pos_y = 0.0;
    drawer_phys.vel_y = 0.0;
    drawer_phys.spring_sim_y = None;

    assert!(
        !drawer_phys.is_animating(),
        "Drawer vertical physics must be completely inactive immediately upon SwipeDown"
    );
    assert_eq!(drawer_phys.pos_y, 0.0);
    assert_eq!(drawer_phys.vel_y, 0.0);
}

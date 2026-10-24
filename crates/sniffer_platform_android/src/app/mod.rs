pub mod actions;
pub mod bridge;
pub mod events;
pub mod frame;
pub mod physics_sync;
pub mod render_thread;
pub mod state;

pub use bridge::{AndroidHostBridge, RenderMessage, SharedRenderState};
pub use physics_sync::ScrollPhysics;
pub use state::{AppState, KineticScroll};

use android_activity::AndroidApp;
use sniffer_core::ScreenMetrics;
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[allow(clippy::pedantic, clippy::too_many_lines)]
pub fn android_main(app: AndroidApp) {
    unsafe {
        std::env::set_var("RUST_BACKTRACE", "full");
    }
    sniffer_render::draw::elements::set_icon_loader(crate::jni::bridge::request_async_app_icon);
    sniffer_plugin::js::host_bridge::set_host_bridge(Box::new(AndroidHostBridge));

    crate::jni::bridge::init_app_list_cache();
    crate::jni::bridge::init_icon_worker_pool();
    crate::set_android_app(&app);
    crate::jni::bridge::set_show_wallpaper_flag(&app);

    let (density, scaled_density, disp_w, disp_h) = crate::jni::get_display_metrics();
    let refresh_rate = crate::jni::get_display_refresh_rate();

    let (initial_width, initial_height) = app.native_window().map_or((disp_w, disp_h), |window| {
        let w = f32::from(u16::try_from(window.width()).unwrap_or(0));
        let h = f32::from(u16::try_from(window.height()).unwrap_or(0));
        if w > 0.0 && h > 0.0 {
            (w, h)
        } else {
            (disp_w, disp_h)
        }
    });

    // Initialize screen dimensions BEFORE AppState evaluates JS plugins
    // so that the initial UI layout has the correct vw/vh values.
    sniffer_core::types::SCREEN_WIDTH.store(
        initial_width.to_bits(),
        std::sync::atomic::Ordering::Relaxed,
    );
    sniffer_core::types::SCREEN_HEIGHT.store(
        initial_height.to_bits(),
        std::sync::atomic::Ordering::Relaxed,
    );

    let mut state = AppState::new(&app);
    state.cached_density = (density, scaled_density);
    state.cached_screen_size = (initial_width, initial_height);
    state.refresh_rate = refresh_rate;
    state.idle_detector.set_refresh_rate(refresh_rate);
    if let Ok(mut prof) = state.profiler.lock() {
        prof.watchdog.set_refresh_rate(refresh_rate);
    }

    sniffer_core::dev_log!(
        "[Display] Display: {initial_width}x{initial_height} @ {refresh_rate} Hz (density: {density})"
    );

    let initial_root = sniffer_core::types::Element::Container {
        id: None,
        style: sniffer_core::style::Style::default(),
        children: vec![],
    };
    let initial_layout = Arc::new(sniffer_core::layout::LayoutNode::new(
        initial_root.clone(),
        sniffer_core::Rect {
            x: 0.0,
            y: 0.0,
            width: initial_width,
            height: initial_height,
        },
    ));

    let initial_root_arc = Arc::new(initial_root);
    let mut root_element = (*initial_root_arc).clone();
    state.cached_root_element = Arc::clone(&initial_root_arc);

    let init_time = std::time::Instant::now();
    let shared_render_state = Arc::new(RwLock::new(Arc::new(SharedRenderState {
        root_element: initial_root_arc,
        layout_tree: initial_layout,
        metrics: ScreenMetrics::from_scale(initial_width, initial_height, density, scaled_density),
        style_map: state.style_map.clone(),
        data_map: state.data_map.clone(),
        transition_manager: state.transition_manager.clone(),
        shaders_warmed_up: false,
        version: 0,
        frame_start: init_time,
        layout_end: init_time,
        scroll_offsets: std::collections::HashMap::new(),
        page_indicator_float: None,
    })));

    let (render_tx, render_rx) = crossbeam_channel::unbounded::<RenderMessage>();
    let (vsync_tx, vsync_rx) = crossbeam_channel::bounded::<()>(1);

    render_thread::spawn_render_thread(
        app.clone(),
        shared_render_state.clone(),
        state.action_queue.clone(),
        state.profiler.clone(),
        Arc::clone(&state.pkg_registry),
        render_rx,
        vsync_tx,
    );

    let mut last_frame_time = std::time::Instant::now();

    while state.running {
        let has_active_physics = !state.kinetic_scrolls.is_empty()
            || state
                .scroll_physics
                .values()
                .any(ScrollPhysics::is_animating);

        let has_active_transitions = state.transition_manager.is_animating();
        let is_dragging = state.active_scrollview_drag.is_some();
        let has_active_animation = has_active_physics || has_active_transitions || is_dragging;

        let poll_timeout = Some(state.recommended_poll_timeout(has_active_animation));

        let needs_redraw = events::poll_and_handle_events(
            &app,
            &mut state,
            &mut root_element,
            &render_tx,
            poll_timeout,
        );

        let frame_start = std::time::Instant::now();
        let raw_elapsed = frame_start.duration_since(last_frame_time).as_secs_f32();
        let target_frame_interval = 1.0 / state.refresh_rate.clamp(30.0, 240.0);
        let dt = if raw_elapsed > 0.050 {
            target_frame_interval
        } else {
            raw_elapsed.clamp(0.001, 0.033)
        };
        last_frame_time = frame_start;

        if crate::jni::bridge::apps::take_app_list_updated() {
            if let Ok(apps) = crate::jni::get_application_list() {
                let pkg_names: Vec<String> = apps.iter().map(|a| a.package_name.clone()).collect();
                crate::jni::bridge::prefetch_app_icons(&pkg_names);
                let _ = state
                    .plugin_registry
                    .vault()
                    .set_json("system.apps", &apps, "system");
                state
                    .plugin_registry
                    .broadcast("vault.changed:system.apps", "{}");
                state.plugin_registry.broadcast("system.apps", "{}");
                state.cached_layout = None;
                state.layout_dirty = true;
            }
        }

        let has_pending_actions = state.action_queue.lock().is_ok_and(|q| !q.is_empty());
        let has_active_event = needs_redraw
            || has_active_animation
            || has_pending_actions
            || !state.event_bus.is_empty();
        if state.should_tick(state.last_tick_time, has_active_event) {
            let tick_dt = state.last_tick_time.elapsed().as_secs_f32();
            state.last_tick_time = std::time::Instant::now();

            state.plugin_registry.tick();
            if let Ok(mut reg) = state.pkg_registry.write() {
                reg.tick_all(tick_dt);
            }
        }

        let current_ui_version =
            sniffer_core::types::UI_VERSION.load(std::sync::atomic::Ordering::Relaxed);
        let has_user_events = !state.event_bus.is_empty();
        let is_finger_dragging = is_dragging;
        let ui_changed = (state.layout_dirty || (current_ui_version != state.last_ui_version))
            && (!is_finger_dragging || has_user_events);

        let is_first_frame = matches!(
            &root_element,
            sniffer_core::types::Element::Container { children, .. } if children.is_empty()
        );

        let should_update = needs_redraw
            || ui_changed
            || has_active_physics
            || has_active_transitions
            || is_first_frame
            || state.cached_layout.is_none();

        let has_active_work = should_update || is_dragging || has_pending_actions;
        state.update_idle_state(has_active_work);

        let rendered_this_frame = if should_update {
            // Drain any pending vsync tokens from previous frames before submitting the new frame
            while vsync_rx.try_recv().is_ok() {}

            frame::update_and_render_state(
                &app,
                &mut state,
                &mut root_element,
                &shared_render_state,
                dt,
                ui_changed,
                current_ui_version,
                frame_start,
            );
            let _ = render_tx.send(RenderMessage::RequestRedraw);
            true
        } else {
            false
        };

        let has_active_animation_after = !state.kinetic_scrolls.is_empty()
            || state.transition_manager.is_animating()
            || state.active_scrollview_drag.is_some()
            || state
                .scroll_physics
                .values()
                .any(ScrollPhysics::is_animating);

        if rendered_this_frame && has_active_animation_after {
            // Wait for hardware VSync completion signal from the render thread's swap_buffers().
            // This locks the main thread's physics simulation and frame production in exact 1:1 cadence
            // with the display panel refresh rate (60Hz, 90Hz, 120Hz), eliminating phase drift,
            // skipped frames, and micro-jank.
            // A timeout of 25ms ensures we never deadlock if the window is unbound or paused.
            let _ = vsync_rx.recv_timeout(Duration::from_millis(25));
        } else {
            let elapsed = frame_start.elapsed();
            let target_frame_duration = state.target_frame_duration(has_active_animation_after);
            if elapsed < target_frame_duration {
                std::thread::sleep(target_frame_duration - elapsed);
            }
        }
    }

    if let Ok(mut reg) = state.pkg_registry.write() {
        reg.unload_all();
    }
}

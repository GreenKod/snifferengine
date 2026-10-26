use super::bridge::SharedRenderState;
use super::physics_sync;
use super::state::AppState;
use android_activity::AndroidApp;
use sniffer_core::types::Element;
use sniffer_core::ui::event::UiEvent;
use sniffer_core::{ScreenMetrics, Size, calculate_layout};
use std::sync::{Arc, RwLock};

struct ViewportMetrics {
    width: f32,
    height: f32,
    metrics: ScreenMetrics,
    screen_changed: bool,
    ui_changed: bool,
}

#[allow(
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
fn resolve_viewport_metrics(
    app: &AndroidApp,
    state: &mut AppState,
    mut ui_changed: bool,
) -> Option<ViewportMetrics> {
    let (width, height) = if let Some(window) = app.native_window() {
        (
            f32::from(u16::try_from(window.width()).expect("window width fits in u16")),
            f32::from(u16::try_from(window.height()).expect("window height fits in u16")),
        )
    } else {
        state.layout.cached_screen_size
    };

    if width < 1.0 || height < 1.0 {
        return None;
    }

    let screen_changed = (width - state.layout.cached_screen_size.0).abs() > 0.5
        || (height - state.layout.cached_screen_size.1).abs() > 0.5;

    if screen_changed {
        state.layout.cached_screen_size = (width, height);
        state.layout.cached_density = crate::jni::get_density();
        let insets = crate::jni::get_system_insets(app).unwrap_or_else(|| {
            let (density, _) = state.layout.cached_density;
            let fallback_top = (36.0 * density).round();
            let fallback_bottom = (48.0 * density).round();
            sniffer_core::math::EdgeInsets::new(fallback_top, fallback_bottom, 0.0, 0.0)
        });
        state.layout.cached_safe_area = (insets.top, insets.bottom);
        state.layout.system_insets = insets;

        state.layout.cached_layout = None;
        state.physics.cached_max_scroll.clear();
        state.physics.last_indicator_page = None;

        sniffer_core::types::SCREEN_WIDTH
            .store(width.to_bits(), std::sync::atomic::Ordering::Relaxed);
        sniffer_core::types::SCREEN_HEIGHT
            .store(height.to_bits(), std::sync::atomic::Ordering::Relaxed);

        state
            .runtime
            .plugin_registry
            .broadcast("dock.stateChanged", "{}");
        state.runtime.plugin_registry.broadcast("dock.ready", "{}");
        state
            .runtime
            .plugin_registry
            .broadcast("system.resized", "{}");

        ui_changed = true;
    }

    let (density, scaled_density) = state.layout.cached_density;
    let metrics = ScreenMetrics::from_scale(width, height, density, scaled_density);

    Some(ViewportMetrics {
        width,
        height,
        metrics,
        screen_changed,
        ui_changed,
    })
}

struct TreeSyncTimings {
    #[cfg(feature = "devkit")]
    t_build_ui: f32,
    #[cfg(feature = "devkit")]
    t_sync: f32,
}

#[allow(clippy::similar_names)]
fn sync_ui_tree_and_virtualization(
    state: &mut AppState,
    root_element: &mut Element,
    width: f32,
    height: f32,
    dt: f32,
    ui_changed: bool,
    screen_changed: bool,
    is_first_frame: bool,
) -> (bool, TreeSyncTimings) {
    #[cfg(feature = "devkit")]
    let b0 = std::time::Instant::now();
    if ui_changed || is_first_frame {
        if let Some(new_root) = state.runtime.plugin_registry.build_ui() {
            *root_element = new_root;
            state.layout.cached_root_element = Arc::new(root_element.clone());
            state.layout.layout_dirty = true;
        }
    }
    #[cfg(feature = "devkit")]
    let t_build_ui = if ui_changed || is_first_frame {
        b0.elapsed().as_secs_f32() * 1000.0
    } else {
        0.0_f32
    };

    #[cfg(feature = "devkit")]
    let s0 = std::time::Instant::now();
    if ui_changed || screen_changed || is_first_frame {
        state
            .layout
            .virtual_page_manager
            .on_viewport_resized(width, height, root_element);
        state
            .layout
            .virtual_page_manager
            .virtualize_tree(root_element);
        physics_sync::sync_scroll_physics_from_tree(
            root_element,
            &mut state.physics.scroll_physics,
        );
        state.runtime.transition_manager.sync_tree(root_element);
    }
    #[cfg(feature = "devkit")]
    let t_sync = if ui_changed || screen_changed || is_first_frame {
        s0.elapsed().as_secs_f32() * 1000.0
    } else {
        0.0_f32
    };
    let is_animating = state.runtime.transition_manager.tick(dt)
        || state.runtime.transition_manager.is_animating();

    (
        is_animating,
        TreeSyncTimings {
            #[cfg(feature = "devkit")]
            t_build_ui,
            #[cfg(feature = "devkit")]
            t_sync,
        },
    )
}

struct LayoutResult {
    layout_tree: Arc<sniffer_core::layout::LayoutNode>,
    layout_recalculated: bool,
    #[cfg(feature = "devkit")]
    t_taffy: f32,
}

fn calculate_and_sync_layout(
    state: &mut AppState,
    root_element: &mut Element,
    width: f32,
    height: f32,
    current_ui_version: u64,
) -> LayoutResult {
    if let Some(ref cached) = state.layout.cached_layout {
        let cached_is_landscape = cached.rect.width > cached.rect.height;
        let window_is_landscape = width > height;
        if cached_is_landscape != window_is_landscape {
            state.layout.cached_layout = None;
            state.layout.layout_dirty = true;
            state
                .layout
                .virtual_page_manager
                .invalidate_on_resize(root_element);
        }
    }

    #[cfg(feature = "devkit")]
    let mut t_taffy = 0.0_f32;
    let layout_recalculated = state.layout.layout_dirty || state.layout.cached_layout.is_none();
    let layout_tree = if !state.layout.layout_dirty
        && let Some(ref cached) = state.layout.cached_layout
    {
        cached.clone()
    } else {
        #[cfg(feature = "devkit")]
        let l0 = std::time::Instant::now();
        state.layout.last_ui_version = current_ui_version;
        let fresh = Arc::new(calculate_layout(
            root_element,
            Size::new(width, height),
            0.0,
            0.0,
        ));
        super::state::update_max_scroll_cache(&mut state.physics, root_element, &fresh);
        sniffer_core::physics::sync_max_scroll_from_layout(
            root_element,
            &fresh,
            &mut state.physics.scroll_physics,
        );
        state.layout.cached_layout = Some(fresh.clone());
        state.layout.layout_dirty = false;
        #[cfg(feature = "devkit")]
        {
            t_taffy = l0.elapsed().as_secs_f32() * 1000.0;
        }
        fresh
    };

    LayoutResult {
        layout_tree,
        layout_recalculated,
        #[cfg(feature = "devkit")]
        t_taffy,
    }
}

struct PhysicsStepResult {
    scroll_offsets: std::collections::HashMap<u64, (f32, f32)>,
    kinetic_active: bool,
    physics_active: bool,
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
fn step_physics_and_kinetics(
    state: &mut AppState,
    root_element: &mut Element,
    dt: f32,
) -> PhysicsStepResult {
    let get_max_scroll = |target_id: Option<u64>| -> f32 {
        target_id
            .and_then(|id| state.physics.cached_max_scroll.get(&id).copied())
            .unwrap_or(0.0)
    };

    let kinetic_active = !state.physics.kinetic_scrolls.is_empty();
    state.physics.kinetic_scrolls.retain_mut(|k| {
        if k.velocity_x.abs() > 0.5 || k.velocity_y.abs() > 0.5 {
            let (step_x, next_vx) =
                sniffer_core::physics::step_momentum_decay(k.velocity_x, 7.62, dt);
            let (step_y, next_vy) =
                sniffer_core::physics::step_momentum_decay(k.velocity_y, 3.6, dt);
            state.runtime.event_bus.push(UiEvent::Scroll(
                Some(k.sv_id),
                step_x,
                step_y,
                999_999.0,
                get_max_scroll(Some(k.sv_id)),
            ));
            k.velocity_x = next_vx;
            k.velocity_y = next_vy;
            true
        } else {
            false
        }
    });

    let mut scroll_offsets = std::collections::HashMap::new();
    let mut physics_active = false;
    let phys_ids: Vec<u64> = state.physics.scroll_physics.keys().copied().collect();
    for sv_id in phys_ids {
        if let Some(phys) = state.physics.scroll_physics.get_mut(&sv_id) {
            let moved = phys.tick(dt);
            if moved || phys.is_dragging {
                physics_active = true;
            }

            if let Some(snap_width) = phys.snap_x {
                if snap_width > 0.0 {
                    let max_page = (phys.page_count.unwrap_or(1) as i32 - 1).max(0);
                    let page_float = (phys.pos_x / snap_width).clamp(0.0, max_page as f32);
                    let clamped_page = page_float.round() as usize;

                    let page_window_changed = state
                        .layout
                        .virtual_page_manager
                        .update_predicted_page(clamped_page, root_element, phys.vel_x);

                    if page_window_changed {
                        state.layout.cached_layout = None;
                    }

                    phys.last_snap_page = clamped_page as i32;

                    if phys.snap_just_completed && phys.on_snap.is_some() {
                        state.runtime.event_bus.push(UiEvent::PageSnapped {
                            widget_id: sv_id,
                            page: clamped_page as i32,
                        });
                    }

                    let target_page_float = if phys.snap_just_completed {
                        clamped_page as f32
                    } else {
                        page_float
                    };

                    let page_moved = phys.is_dragging
                        || phys.vel_x.abs() > 0.001
                        || phys.spring_sim_x.is_some()
                        || phys.snap_just_completed
                        || state
                            .physics
                            .last_indicator_page
                            .is_none_or(|p| (p - target_page_float).abs() > 0.002);

                    if page_moved {
                        state.physics.last_indicator_page = Some(target_page_float);
                        physics_active = true;
                    }
                }
            }

            scroll_offsets.insert(sv_id, (phys.pos_x, phys.pos_y));
        }
    }

    PhysicsStepResult {
        scroll_offsets,
        kinetic_active,
        physics_active,
    }
}

fn dispatch_events_and_actions(state: &AppState, app: &AndroidApp) -> bool {
    let has_events = !state.runtime.event_bus.is_empty();

    state.runtime.plugin_registry.dispatch(
        &state.runtime.event_bus,
        &state.runtime.style_map,
        &state.runtime.data_map,
        &state.runtime.action_queue,
    );

    super::actions::process_android_actions(&state.runtime.action_queue, app);

    has_events
}

#[allow(clippy::too_many_arguments)]
fn publish_render_snapshot_if_needed(
    state: &mut AppState,
    shared_render_state: &Arc<RwLock<Arc<SharedRenderState>>>,
    layout_tree: Arc<sniffer_core::layout::LayoutNode>,
    metrics: ScreenMetrics,
    scroll_offsets: std::collections::HashMap<u64, (f32, f32)>,
    current_warmed_up: bool,
    frame_start: std::time::Instant,
    needs_state_update: bool,
) {
    if needs_state_update {
        let layout_end = std::time::Instant::now();
        state.pacing.render_state_version = state.pacing.render_state_version.wrapping_add(1);
        *shared_render_state.write().unwrap() = Arc::new(SharedRenderState {
            root_element: Arc::clone(&state.layout.cached_root_element),
            layout_tree,
            metrics,
            style_map: state.runtime.style_map.clone(),
            data_map: state.runtime.data_map.clone(),
            transition_manager: state.runtime.transition_manager.clone(),
            shaders_warmed_up: current_warmed_up,
            version: state.pacing.render_state_version,
            frame_start,
            layout_end,
            scroll_offsets,
            page_indicator_float: state.physics.last_indicator_page,
        });
    }
}

#[allow(clippy::similar_names)]
pub fn update_and_render_state(
    app: &AndroidApp,
    state: &mut AppState,
    root_element: &mut Element,
    shared_render_state: &Arc<RwLock<Arc<SharedRenderState>>>,
    dt: f32,
    ui_changed: bool,
    current_ui_version: u64,
    frame_start: std::time::Instant,
) {
    let Some(viewport) = resolve_viewport_metrics(app, state, ui_changed) else {
        return;
    };

    let is_first_frame =
        matches!(root_element, Element::Container { children, .. } if children.is_empty());

    let (is_animating, _timings) = sync_ui_tree_and_virtualization(
        state,
        root_element,
        viewport.width,
        viewport.height,
        dt,
        viewport.ui_changed,
        viewport.screen_changed,
        is_first_frame,
    );

    let layout_res = calculate_and_sync_layout(
        state,
        root_element,
        viewport.width,
        viewport.height,
        current_ui_version,
    );

    #[cfg(feature = "devkit")]
    if _timings.t_build_ui + _timings.t_sync + layout_res.t_taffy > 16.0 {
        sniffer_core::dev_log!(
            "[MainThread Breakdown] Total: {:.1}ms (JS build_ui: {:.1}ms, Tree sync: {:.1}ms, Taffy layout: {:.1}ms)",
            frame_start.elapsed().as_secs_f32() * 1000.0,
            _timings.t_build_ui,
            _timings.t_sync,
            layout_res.t_taffy
        );
    }

    let phys_res = step_physics_and_kinetics(state, root_element, dt);

    let has_events = dispatch_events_and_actions(state, app);

    let current_warmed_up = shared_render_state.read().unwrap().shaders_warmed_up;
    let needs_state_update = viewport.ui_changed
        || viewport.screen_changed
        || state.layout.layout_dirty
        || layout_res.layout_recalculated
        || is_animating
        || phys_res.kinetic_active
        || phys_res.physics_active
        || has_events
        || !current_warmed_up;

    publish_render_snapshot_if_needed(
        state,
        shared_render_state,
        layout_res.layout_tree,
        viewport.metrics,
        phys_res.scroll_offsets,
        current_warmed_up,
        frame_start,
        needs_state_update,
    );
}

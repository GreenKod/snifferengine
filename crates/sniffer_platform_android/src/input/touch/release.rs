use super::scroll::resolve_active_scroll;
use android_activity::InputStatus;
use android_activity::input::MotionEvent;
use sniffer_core::Point;
use sniffer_core::ui::event::UiEvent;
use sniffer_render::draw::{
    find_clicked_button_with_scroll, find_hovered_button_with_scroll, find_hovered_scrollview,
};

#[allow(clippy::too_many_lines)]
pub fn handle_touch_release(
    #[allow(unused_variables)] motion_event: &MotionEvent,
    state: &mut crate::app::AppState,
    root_element: &sniffer_core::types::Element,
    layout_tree: &sniffer_core::layout::LayoutNode,
    point: Point,
) -> InputStatus {
    let now = std::time::Instant::now();
    let mut tracker = sniffer_core::physics::FlingVelocityTracker::new();
    for &(dx, dy, t) in &state.input.drag_history {
        tracker.add_movement(dx, dy, t);
    }
    let (fling_vel_x, fling_vel_y) = tracker.compute_velocity(now);

    let released_sv_id = state.input.active_scrollview_drag;
    if let Some(sv_id) = released_sv_id {
        if let Some(phys) = state.physics.scroll_physics.get_mut(&sv_id) {
            if phys.snap_x.is_some() {
                if state.input.gesture_axis == crate::app::state::GestureAxis::Vertical {
                    phys.release_drag(0.0);
                } else {
                    phys.release_drag(fling_vel_x);
                }
            } else {
                let is_system_nav_gesture = state
                    .layout
                    .is_in_bottom_system_nav(state.input.touch_start_pos.y);
                if is_system_nav_gesture {
                    phys.pos_y = 0.0;
                    phys.vel_y = 0.0;
                    phys.spring_sim_y = None;
                } else if phys.pos_y < 0.0 {
                    // If pulled into elastic overscroll, let gesture classifier decide whether to dismiss or spring back
                    phys.vel_y = 0.0;
                } else {
                    phys.release_drag_y(fling_vel_y);
                }
            }
        } else {
            let momentum_enabled = if let Some((
                sniffer_core::types::Element::ScrollView {
                    momentum_scrolling, ..
                },
                _,
            )) = find_hovered_scrollview(root_element, layout_tree, point)
            {
                momentum_scrolling.unwrap_or(true)
            } else {
                true
            };

            if momentum_enabled {
                state
                    .physics
                    .kinetic_scrolls
                    .push(crate::app::KineticScroll {
                        sv_id,
                        velocity_x: fling_vel_x,
                        velocity_y: fling_vel_y,
                    });
            }
        }
    }

    state.input.active_scrollview_drag = None;
    state.input.last_drag_delta = (0.0, 0.0);
    state.input.drag_history.clear();

    let prev_hovered = state.input.hovered_btn.take();
    let hovered_data =
        find_hovered_button_with_scroll(root_element, layout_tree, point, &|id_opt, sx, sy| {
            resolve_active_scroll(&state.physics.scroll_physics, id_opt, sx, sy)
        });
    let released_btn = hovered_data.map(|(id, _)| id);
    if let Some(prev) = prev_hovered {
        state.runtime.event_bus.push(UiEvent::HoverEnd(prev));
    }

    state
        .runtime
        .event_bus
        .push(UiEvent::PointerUp(released_btn));

    let density = state.layout.cached_density.0.max(1.0);
    let is_static_tap = crate::input::gestures::GestureClassifier::is_tap(
        state.input.touch_start_pos,
        point,
        density,
    );

    #[cfg(feature = "devkit")]
    {
        let pointer_count = motion_event.pointers().count();
        let gesture_name = if is_static_tap {
            "TAP (Release)".to_string()
        } else {
            "SWIPE (Release)".to_string()
        };
        let target_name =
            hovered_data.map_or_else(|| "None".to_string(), |(btn, _)| format!("btn_{btn:x}"));

        if let Ok(mut prof) = state.pacing.profiler.lock() {
            prof.touch_telemetry.active_pointers = pointer_count;
            prof.touch_telemetry.gesture = gesture_name;
            prof.touch_telemetry.target_element = target_name;
            prof.touch_telemetry.touch_x = point.x;
            prof.touch_telemetry.touch_y = point.y;
        }
    }

    if is_static_tap {
        let clicked = find_clicked_button_with_scroll(
            root_element,
            layout_tree,
            state.input.touch_start_pos,
            &|id_opt, sx, sy| resolve_active_scroll(&state.physics.scroll_physics, id_opt, sx, sy),
        )
        .or_else(|| {
            find_clicked_button_with_scroll(root_element, layout_tree, point, &|id_opt, sx, sy| {
                resolve_active_scroll(&state.physics.scroll_physics, id_opt, sx, sy)
            })
        });

        if let Some((clicked_btn, rect)) = clicked {
            sniffer_core::dev_log!(
                "[Input] TAP on button id: {clicked_btn} (hex: {clicked_btn:x})"
            );
            state
                .runtime
                .event_bus
                .push(UiEvent::Click(clicked_btn, rect.width, rect.height));
        } else {
            sniffer_core::dev_log!(
                "[Input] TAP clicked outside (no button at start={:?}, end={:?})",
                state.input.touch_start_pos,
                point
            );
            state.runtime.event_bus.push(UiEvent::ClickOutside);
        }
    } else {
        let current_pos_y = released_sv_id
            .and_then(|id| state.physics.scroll_physics.get(&id))
            .map_or(0.0, |p| p.pos_y);
        let was_scrolled_down = state.input.active_scrollview_start_y > 30.0 * density
            && current_pos_y > 10.0 * density;
        let swipe_gesture = crate::input::gestures::GestureClassifier::classify_vertical_swipe(
            state.input.touch_start_pos,
            point,
            fling_vel_y,
            density,
            state.input.gesture_axis,
            was_scrolled_down,
        );

        match swipe_gesture {
            Some(crate::input::gestures::VerticalSwipeGesture::SwipeUp) => {
                let is_bottom_nav_gesture = state
                    .layout
                    .is_in_bottom_system_nav(state.input.touch_start_pos.y);
                if is_bottom_nav_gesture {
                    // Upward swipe starting from the bottom system navigation bar is an OS Home/Dismiss gesture
                    state.runtime.event_bus.push(UiEvent::SwipeDown);
                    if let Some(sv_id) = released_sv_id {
                        if let Some(phys) = state.physics.scroll_physics.get_mut(&sv_id) {
                            phys.pos_y = 0.0;
                            phys.vel_y = 0.0;
                            phys.spring_sim_y = None;
                        }
                    }
                } else {
                    state.runtime.event_bus.push(UiEvent::SwipeUp);
                }
            }
            Some(crate::input::gestures::VerticalSwipeGesture::SwipeDown) => {
                if let Some(sv_id) = released_sv_id {
                    if let Some(phys) = state.physics.scroll_physics.get_mut(&sv_id) {
                        let pulled_displacement = (-phys.pos_y).max(0.0);
                        phys.pos_y = 0.0;
                        phys.vel_y = 0.0;
                        phys.spring_sim_y = None;

                        let action_name = phys
                            .dismiss_config
                            .as_ref()
                            .filter(|cfg| {
                                cfg.direction == sniffer_core::types::DismissDirection::Down
                            })
                            .and_then(|cfg| {
                                if let Some(target_id) = cfg.target_container_id.as_deref() {
                                    let screen_h = f32::from_bits(
                                        sniffer_core::types::SCREEN_HEIGHT
                                            .load(std::sync::atomic::Ordering::Relaxed),
                                    );
                                    if let Some(anim) =
                                        state.runtime.transition_manager.states.get_mut(target_id)
                                    {
                                        anim.start_style.transform.translate_y =
                                            pulled_displacement;
                                        anim.current_style.transform.translate_y =
                                            pulled_displacement;
                                        anim.target_style.transform.translate_y = screen_h;
                                        anim.target_style.opacity = 0.0;
                                        anim.target_style.transition.duration = 0.22;
                                        anim.target_style.transition.easing =
                                            sniffer_core::style::Easing::EaseOut;
                                        anim.time_elapsed = 0.0;
                                        anim.is_active = true;
                                    }
                                }
                                cfg.action_event.clone()
                            });

                        state.runtime.event_bus.push(UiEvent::ScrollDismiss {
                            widget_id: sv_id,
                            displacement: pulled_displacement,
                            velocity: fling_vel_y,
                            action: action_name,
                        });
                    }
                }

                state.runtime.event_bus.push(UiEvent::SwipeDown);

                if let Some(sv_id) = released_sv_id {
                    for (&id, phys) in &mut state.physics.scroll_physics {
                        if id != sv_id && phys.snap_x.is_none() {
                            phys.pos_y = 0.0;
                            phys.vel_y = 0.0;
                            phys.spring_sim_y = None;
                        }
                    }
                }
            }
            None => {
                for phys in state.physics.scroll_physics.values_mut() {
                    if phys.pos_y < 0.0 {
                        phys.release_drag_y(0.0);
                    }
                }
            }
        }
        state.runtime.event_bus.push(UiEvent::ClickOutside);
    }
    state.input.active_scrollview_start_y = 0.0;
    state.input.gesture_axis = crate::app::state::GestureAxis::Undetermined;
    InputStatus::Handled
}

pub fn handle_touch_cancel(state: &mut crate::app::AppState) -> InputStatus {
    let active_id = state.input.active_scrollview_drag.take();
    if let Some(sv_id) = active_id {
        if let Some(phys) = state.physics.scroll_physics.get_mut(&sv_id) {
            if phys.snap_x.is_some() {
                phys.release_drag(0.0);
            } else {
                phys.release_drag_y(0.0);
            }
        }
    }
    for phys in state.physics.scroll_physics.values_mut() {
        if phys.is_dragging || phys.pos_y < 0.0 {
            if phys.snap_x.is_some() {
                phys.release_drag(0.0);
            } else {
                phys.release_drag_y(0.0);
            }
        }
    }
    state.input.active_scrollview_start_y = 0.0;
    state.input.gesture_axis = crate::app::state::GestureAxis::Undetermined;
    state.input.drag_history.clear();
    let prev_hovered = state.input.hovered_btn.take();
    if let Some(prev) = prev_hovered {
        state.runtime.event_bus.push(UiEvent::HoverEnd(prev));
    }
    InputStatus::Handled
}

pub fn handle_touch_wheel_scroll(
    motion_event: &MotionEvent,
    state: &mut crate::app::AppState,
    root_element: &sniffer_core::types::Element,
    layout_tree: &sniffer_core::layout::LayoutNode,
    point: Point,
) -> InputStatus {
    let pointer = motion_event.pointer_at_index(motion_event.pointer_index());
    let axis_v = pointer.axis_value(android_activity::input::Axis::Vscroll);
    let axis_h = pointer.axis_value(android_activity::input::Axis::Hscroll);

    if let Some((
        sniffer_core::types::Element::ScrollView {
            id,
            scroll_sensitivity,
            dynamic_sensitivity,
            ..
        },
        lay,
    )) = find_hovered_scrollview(root_element, layout_tree, point)
    {
        let mut factor = scroll_sensitivity.unwrap_or(1.0);
        if dynamic_sensitivity.unwrap_or(false) && !lay.children.is_empty() {
            let view_height = lay.rect.height;
            let mut min_y = f32::MAX;
            let mut max_y = f32::MIN;
            for child in &lay.children {
                if child.rect.y < min_y {
                    min_y = child.rect.y;
                }
                if child.rect.y + child.rect.height > max_y {
                    max_y = child.rect.y + child.rect.height;
                }
            }
            let content_height = max_y - min_y;
            if view_height > 0.0 && content_height > view_height {
                factor *= content_height / view_height;
            }
        }

        let sv_id = id
            .as_deref()
            .map(|id_str| sniffer_core::ui::widget::fnv1a(id_str.as_bytes()));
        state.runtime.event_bus.push(UiEvent::Scroll(
            sv_id,
            axis_h * -20.0 * factor,
            axis_v * -20.0 * factor,
            999_999.0,
            state.get_max_scroll(sv_id),
        ));
    }
    InputStatus::Handled
}

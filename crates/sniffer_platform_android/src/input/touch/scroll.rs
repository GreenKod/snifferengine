use android_activity::InputStatus;
use android_activity::input::{MotionAction, MotionEvent};
use sniffer_core::Point;
use sniffer_core::ui::event::UiEvent;
use sniffer_render::draw::{find_clicked_button_with_scroll, find_hovered_scrollview};

#[allow(clippy::implicit_hasher)]
pub fn resolve_active_scroll(
    scroll_physics: &std::collections::HashMap<u64, crate::app::physics_sync::ScrollPhysics>,
    id_opt: Option<&str>,
    default_x: f32,
    default_y: f32,
) -> (f32, f32) {
    if let Some(id_str) = id_opt {
        let sv_id = sniffer_core::ui::widget::fnv1a(id_str.as_bytes());
        if let Some(phys) = scroll_physics.get(&sv_id) {
            return (phys.pos_x, phys.pos_y);
        }
    }
    (default_x, default_y)
}

#[allow(clippy::too_many_lines)]
pub fn handle_touch_move_or_down(
    motion_event: &MotionEvent,
    state: &mut crate::app::AppState,
    root_element: &sniffer_core::types::Element,
    layout_tree: &sniffer_core::layout::LayoutNode,
    point: Point,
    delta_x: f32,
    delta_y: f32,
) -> InputStatus {
    if motion_event.action() == MotionAction::Down
        || motion_event.action() == MotionAction::PointerDown
    {
        if let Some((
            sniffer_core::types::Element::ScrollView {
                id, capture_drag, ..
            },
            _,
        )) = find_hovered_scrollview(root_element, layout_tree, point)
            && capture_drag.unwrap_or(true)
        {
            let sv_id = id
                .as_deref()
                .map(|id_str| sniffer_core::ui::widget::fnv1a(id_str.as_bytes()));
            let screen_h = f32::from_bits(
                sniffer_core::types::SCREEN_HEIGHT.load(std::sync::atomic::Ordering::Relaxed),
            );
            let drawer_id = sniffer_core::ui::widget::fnv1a(b"app_drawer_scroll");
            let is_bottom_nav_area = screen_h > 0.0 && point.y > screen_h * 0.93;
            if sv_id == Some(drawer_id) && is_bottom_nav_area {
                state.input.active_scrollview_drag = None;
            } else {
                state.input.active_scrollview_drag = sv_id;
                if let Some(id_val) = sv_id {
                    if let Some(phys) = state.physics.scroll_physics.get_mut(&id_val) {
                        phys.vel_x = 0.0;
                        phys.vel_y = 0.0;
                        phys.spring_sim_x = None;
                        phys.spring_sim_y = None;
                        phys.snap_target_x = None;
                        phys.is_dragging = true;
                        phys.drag_start_x = phys.pos_x;
                    }
                }
            }
        }
        state.physics.kinetic_scrolls.clear();
        state.input.drag_history.clear();
        state.input.active_scrollview_start_y = state
            .input
            .active_scrollview_drag
            .and_then(|id| state.physics.scroll_physics.get(&id))
            .map_or(0.0, |p| p.pos_y);

        if let Some((clicked_btn, _)) =
            find_clicked_button_with_scroll(root_element, layout_tree, point, &|id_opt, sx, sy| {
                resolve_active_scroll(&state.physics.scroll_physics, id_opt, sx, sy)
            })
        {
            state
                .runtime
                .event_bus
                .push(UiEvent::PointerDown(clicked_btn));
        } else {
            state.runtime.event_bus.push(UiEvent::PointerDown(0));
        }
    }

    if motion_event.action() == MotionAction::Move {
        state.input.last_drag_delta = (delta_x, delta_y);
        state
            .input
            .drag_history
            .push_back((delta_x, delta_y, std::time::Instant::now()));
        if state.input.drag_history.len() > 16 {
            state.input.drag_history.pop_front();
        }

        let density = state.layout.cached_density.0.max(1.0);
        let total_dx = point.x - state.input.touch_start_pos.x;
        let total_dy = point.y - state.input.touch_start_pos.y;

        // Disambiguate gesture axis once threshold is crossed
        if state.input.gesture_axis == crate::app::state::GestureAxis::Undetermined {
            if total_dx.abs() > 14.0 * density && total_dx.abs() > total_dy.abs() * 1.2 {
                state.input.gesture_axis = crate::app::state::GestureAxis::Horizontal;
            } else if total_dy.abs() > 16.0 * density && total_dy.abs() > total_dx.abs() * 1.35 {
                state.input.gesture_axis = crate::app::state::GestureAxis::Vertical;
            }
        }

        if state.input.active_scrollview_drag.is_none() {
            if let Some((
                sniffer_core::types::Element::ScrollView {
                    id, capture_drag, ..
                },
                _,
            )) = find_hovered_scrollview(root_element, layout_tree, point)
            {
                if capture_drag.unwrap_or(true) {
                    let sv_id = id
                        .as_deref()
                        .map(|id_str| sniffer_core::ui::widget::fnv1a(id_str.as_bytes()));
                    let screen_h = f32::from_bits(
                        sniffer_core::types::SCREEN_HEIGHT
                            .load(std::sync::atomic::Ordering::Relaxed),
                    );
                    let drawer_id = sniffer_core::ui::widget::fnv1a(b"app_drawer_scroll");
                    let is_bottom_nav_area =
                        screen_h > 0.0 && state.input.touch_start_pos.y > screen_h * 0.93;
                    if !(sv_id == Some(drawer_id) && is_bottom_nav_area) {
                        state.input.active_scrollview_drag = sv_id;
                    }
                }
            }
        }

        if let Some(sv_id) = state.input.active_scrollview_drag {
            let phys = state.physics.scroll_physics.entry(sv_id).or_default();
            if phys.snap_x.is_some() {
                // If gesture is NOT locked to vertical, allow horizontal drag with 0 deadzone!
                if state.input.gesture_axis != crate::app::state::GestureAxis::Vertical {
                    phys.apply_drag(delta_x);
                    phys.snap_target_x = None;
                    phys.is_dragging = true;
                }
            } else {
                let drawer_id = sniffer_core::ui::widget::fnv1a(b"app_drawer_scroll");
                if sv_id == drawer_id {
                    let density = state.layout.cached_density.0.max(1.0);
                    let max_pull_down = -180.0 * density;
                    if phys.pos_y <= 0.0 && delta_y < 0.0 {
                        // Natural rubber-band pull-down at top of drawer ("sadece biraz daha gelsin")
                        let new_pos = phys.pos_y + delta_y * 0.60;
                        phys.pos_y = new_pos.max(max_pull_down);
                        phys.vel_y = 0.0;
                        phys.spring_sim_y = None;
                    } else {
                        phys.apply_drag_y(delta_y);
                    }
                    phys.is_dragging = true;
                } else {
                    phys.apply_drag_y(delta_y);
                    phys.is_dragging = true;
                }
            }
        }
    }

    // Hover management:
    // Touchscreens do not support cursor hover. Suppress expensive recursive
    // layout tree hit-testing and avoid spamming QuickJS runtimes with 120Hz
    // UiEvent::Hover messages during touch moves.
    if let Some(prev) = state.input.hovered_btn.take() {
        state.runtime.event_bus.push(UiEvent::HoverEnd(prev));
    }

    #[cfg(feature = "devkit")]
    let target_name = if let Some(sv_id) = state.input.active_scrollview_drag {
        format!("sv_{sv_id:x}")
    } else {
        "None".to_string()
    };

    #[cfg(feature = "devkit")]
    {
        let pointer_count = motion_event.pointers().count();
        let gesture_name = match motion_event.action() {
            MotionAction::Down | MotionAction::PointerDown => "DOWN",
            MotionAction::Move => {
                if pointer_count > 1 {
                    "MULTI_TOUCH / PINCH"
                } else {
                    "DRAG / PAN"
                }
            }
            _ => "UNKNOWN",
        }
        .to_string();

        if let Ok(mut prof) = state.pacing.profiler.lock() {
            prof.touch_telemetry.active_pointers = pointer_count;
            prof.touch_telemetry.gesture = gesture_name;
            prof.touch_telemetry.target_element = target_name;
            prof.touch_telemetry.touch_x = point.x;
            prof.touch_telemetry.touch_y = point.y;
        }
    }

    InputStatus::Handled
}

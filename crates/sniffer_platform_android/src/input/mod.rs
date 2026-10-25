pub mod gestures;

#[cfg(target_os = "android")]
pub mod key;
#[cfg(target_os = "android")]
pub mod touch;

#[cfg(target_os = "android")]
use android_activity::InputStatus;
#[cfg(target_os = "android")]
use android_activity::input::InputEvent;

#[cfg(target_os = "android")]
pub fn handle_input_event(
    input_event: &InputEvent,
    state: &mut super::app::AppState,
    root_element: &sniffer_core::types::Element,
    layout_tree: &sniffer_core::layout::LayoutNode,
) -> InputStatus {
    match input_event {
        InputEvent::MotionEvent(motion_event) => {
            touch::handle_motion_event(motion_event, state, root_element, layout_tree)
        }
        InputEvent::KeyEvent(key_event) => key::handle_key_event(key_event, state),
        _ => InputStatus::Unhandled,
    }
}

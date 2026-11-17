use crate::ui::widget::WidgetId;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UiEvent {
    Click(u64, f32, f32),
    ClickOutside,
    Hover(u64, f32, f32, f32, f32),
    HoverEnd(u64),
    TextInput(String),
    Backspace,
    PointerDown(u64),
    PointerUp(Option<u64>),
    Scroll(Option<u64>, f32, f32, f32, f32),
    WindowResized(f32, f32),
    /// Rust scroll physics: a page snapping animation has completed.
    /// `widget_id`: FNV-1a hash ID of the ScrollView, `page`: 0-based page index.
    PageSnapped {
        widget_id: u64,
        page: i32,
    },
    /// Gesture: Vertical swipe upwards (e.g. to open bottom floor/drawer).
    SwipeUp,
    /// Gesture: Vertical swipe downwards (e.g. to return to top floor/home).
    SwipeDown,
    /// Hardware or system back navigation button / gesture.
    Back,
    /// Generic scroll container dismiss gesture triggered by overscroll.
    ScrollDismiss {
        widget_id: u64,
        displacement: f32,
        velocity: f32,
        action: Option<String>,
    },
}

impl UiEvent {
    /// Returns the widget this event is scoped to, or `None` for global events.
    ///
    /// Widget-scoped events can be routed directly to the plugins that subscribed to
    /// the target ID. Global events (keyboard input, back navigation, window resize,
    /// gestures, and pointer releases without a target) must be delivered to every
    /// interested listener.
    #[must_use]
    pub fn widget_id(&self) -> Option<WidgetId> {
        match self {
            Self::Click(id, ..)
            | Self::Hover(id, ..)
            | Self::HoverEnd(id)
            | Self::PointerDown(id)
            | Self::PointerUp(Some(id))
            | Self::Scroll(Some(id), ..) => Some(*id),
            Self::PageSnapped { widget_id, .. } | Self::ScrollDismiss { widget_id, .. } => {
                Some(*widget_id)
            }
            Self::ClickOutside
            | Self::TextInput(_)
            | Self::Backspace
            | Self::PointerUp(None)
            | Self::Scroll(None, ..)
            | Self::WindowResized(..)
            | Self::SwipeUp
            | Self::SwipeDown
            | Self::Back => None,
        }
    }
}

#[derive(Default)]
pub struct EventBus {
    events: RefCell<Vec<UiEvent>>,
}

impl EventBus {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.borrow().is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.borrow().len()
    }

    pub fn push(&self, event: UiEvent) {
        self.events.borrow_mut().push(event);
    }

    #[must_use]
    pub fn drain(&self) -> Vec<UiEvent> {
        let mut borrowed = self.events.borrow_mut();
        std::mem::take(&mut borrowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_scoped_events_expose_their_target() {
        assert_eq!(UiEvent::Click(1, 0.0, 0.0).widget_id(), Some(1));
        assert_eq!(UiEvent::Hover(2, 0.0, 0.0, 1.0, 1.0).widget_id(), Some(2));
        assert_eq!(UiEvent::HoverEnd(3).widget_id(), Some(3));
        assert_eq!(UiEvent::PointerDown(4).widget_id(), Some(4));
        assert_eq!(UiEvent::PointerUp(Some(5)).widget_id(), Some(5));
        assert_eq!(
            UiEvent::Scroll(Some(6), 1.0, 2.0, 3.0, 4.0).widget_id(),
            Some(6)
        );
        assert_eq!(
            UiEvent::PageSnapped {
                widget_id: 7,
                page: 0
            }
            .widget_id(),
            Some(7)
        );
        assert_eq!(
            UiEvent::ScrollDismiss {
                widget_id: 8,
                displacement: 10.0,
                velocity: 1.0,
                action: None
            }
            .widget_id(),
            Some(8)
        );
    }

    #[test]
    fn global_events_have_no_widget_target() {
        assert_eq!(UiEvent::ClickOutside.widget_id(), None);
        assert_eq!(UiEvent::TextInput("a".to_string()).widget_id(), None);
        assert_eq!(UiEvent::Backspace.widget_id(), None);
        assert_eq!(UiEvent::PointerUp(None).widget_id(), None);
        assert_eq!(UiEvent::Scroll(None, 0.0, 1.0, 0.0, 0.0).widget_id(), None);
        assert_eq!(UiEvent::WindowResized(100.0, 200.0).widget_id(), None);
        assert_eq!(UiEvent::SwipeUp.widget_id(), None);
        assert_eq!(UiEvent::SwipeDown.widget_id(), None);
        assert_eq!(UiEvent::Back.widget_id(), None);
    }
}

use sniffer_core::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GestureAxis {
    #[default]
    Undetermined,
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalSwipeGesture {
    SwipeUp,
    SwipeDown,
}

pub struct GestureClassifier;

impl GestureClassifier {
    /// Determines whether a touch movement qualifies as a static tap based on distance threshold.
    #[must_use]
    pub fn is_tap(start_pos: Point, end_pos: Point, density: f32) -> bool {
        let tap_threshold = 48.0 * density.max(1.0);
        let tap_dist = (end_pos.x - start_pos.x).hypot(end_pos.y - start_pos.y);
        tap_dist < tap_threshold
    }

    /// Classifies vertical gestures into `SwipeUp` or `SwipeDown`, respecting axis locking and scroll boundaries.
    #[must_use]
    pub fn classify_vertical_swipe(
        start_pos: Point,
        end_pos: Point,
        fling_vel_y: f32,
        density: f32,
        gesture_axis: GestureAxis,
        was_scrolled_down: bool,
    ) -> Option<VerticalSwipeGesture> {
        let dy = end_pos.y - start_pos.y;
        let dx = end_pos.x - start_pos.x;
        let vel_y_down = -fling_vel_y;

        let swipe_up_thresh = 50.0 * density.max(1.0);
        let swipe_down_thresh = 60.0 * density.max(1.0);
        let flick_thresh = 28.0 * density.max(1.0);

        let is_vertical = gesture_axis == GestureAxis::Vertical
            || (gesture_axis == GestureAxis::Undetermined && dy.abs() > dx.abs() * 1.15);

        if !is_vertical {
            return None;
        }

        if dy < 0.0 {
            // SwipeUp: Open drawer from home screen
            let upward_dist = -dy;
            let is_flick = upward_dist > flick_thresh && vel_y_down < -180.0;
            let is_drag = upward_dist >= swipe_up_thresh;
            if is_flick || is_drag {
                return Some(VerticalSwipeGesture::SwipeUp);
            }
        } else if !was_scrolled_down {
            // SwipeDown: Close drawer back to home screen
            // Only allowed when NOT scrolled down within the list of apps!
            let downward_dist = dy;
            let is_flick = downward_dist > flick_thresh && vel_y_down > 180.0;
            let is_drag = downward_dist >= swipe_down_thresh;
            if is_flick || is_drag {
                return Some(VerticalSwipeGesture::SwipeDown);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tap_detection() {
        let start = Point::new(100.0, 100.0);
        let end_within = Point::new(110.0, 105.0);
        let end_outside = Point::new(100.0, 200.0);

        assert!(GestureClassifier::is_tap(start, end_within, 2.0));
        assert!(!GestureClassifier::is_tap(start, end_outside, 2.0));
    }

    #[test]
    fn test_swipe_up_flick_and_drag() {
        let start = Point::new(300.0, 800.0);
        let end_flick = Point::new(300.0, 680.0); // 120px up (>= 50*2=100)
        let end_drag = Point::new(300.0, 500.0); // 300px up (>= 120*2=240)

        // Flick with high upward velocity (fling_vel_y = 600, vel_y_down = -600)
        let res_flick = GestureClassifier::classify_vertical_swipe(
            start,
            end_flick,
            600.0,
            2.0,
            GestureAxis::Vertical,
            false,
        );
        assert_eq!(res_flick, Some(VerticalSwipeGesture::SwipeUp));

        // Drag with 0 velocity
        let res_drag = GestureClassifier::classify_vertical_swipe(
            start,
            end_drag,
            0.0,
            2.0,
            GestureAxis::Vertical,
            false,
        );
        assert_eq!(res_drag, Some(VerticalSwipeGesture::SwipeUp));
    }

    #[test]
    fn test_swipe_down_blocked_when_scrolled() {
        let start = Point::new(300.0, 400.0);
        let end = Point::new(300.0, 800.0); // 400px down

        // When scrolled down > 10px, SwipeDown MUST be blocked
        let res_blocked = GestureClassifier::classify_vertical_swipe(
            start,
            end,
            -600.0, // vel_y_down = 600
            2.0,
            GestureAxis::Vertical,
            true, // was_scrolled_down = true
        );
        assert_eq!(res_blocked, None);

        // When at top, SwipeDown is allowed
        let res_allowed = GestureClassifier::classify_vertical_swipe(
            start,
            end,
            -600.0,
            2.0,
            GestureAxis::Vertical,
            false,
        );
        assert_eq!(res_allowed, Some(VerticalSwipeGesture::SwipeDown));
    }

    #[test]
    fn test_horizontal_axis_rejected() {
        let start = Point::new(300.0, 400.0);
        let end = Point::new(300.0, 800.0);

        let res = GestureClassifier::classify_vertical_swipe(
            start,
            end,
            -600.0,
            2.0,
            GestureAxis::Horizontal, // Locked to horizontal
            false,
        );
        assert_eq!(res, None);
    }

    #[test]
    fn test_swipe_down_natural_flick_and_drag() {
        let start = Point::new(300.0, 400.0);
        let end_flick = Point::new(300.0, 490.0); // 90px down (>= 40*2=80)
        let end_drag = Point::new(300.0, 570.0); // 170px down (>= 80*2=160)

        // Moderate flick velocity (fling_vel_y = -280.0, vel_y_down = 280.0 > 250.0)
        let res_flick = GestureClassifier::classify_vertical_swipe(
            start,
            end_flick,
            -280.0,
            2.0,
            GestureAxis::Vertical,
            false,
        );
        assert_eq!(res_flick, Some(VerticalSwipeGesture::SwipeDown));

        // Drag with 0 velocity
        let res_drag = GestureClassifier::classify_vertical_swipe(
            start,
            end_drag,
            0.0,
            2.0,
            GestureAxis::Vertical,
            false,
        );
        assert_eq!(res_drag, Some(VerticalSwipeGesture::SwipeDown));
    }

    #[test]
    fn test_swipe_down_diagonal_thumb_arc() {
        // Natural thumb swipe: dy = 120, dx = 95 -> ratio = 120/95 = 1.263 > 1.15
        let start = Point::new(300.0, 400.0);
        let end_diagonal = Point::new(395.0, 520.0);

        let res = GestureClassifier::classify_vertical_swipe(
            start,
            end_diagonal,
            -300.0,
            2.0,
            GestureAxis::Undetermined,
            false,
        );
        assert_eq!(res, Some(VerticalSwipeGesture::SwipeDown));
    }
}


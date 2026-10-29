//! # SnifferEngine
//!
//! General-purpose, declarative 120Hz embedded UI and graphics engine.
//! Unifies layout (`sniffer_core`), GPU rendering (`sniffer_render`), QuickJS plugin
//! execution (`sniffer_plugin`), package management (`sniffer_pkg`), and platform backends
//! (`sniffer_platform_desktop` / `sniffer_platform_android`) under a single facade crate.

pub use sniffer_core as core;
pub use sniffer_pkg as pkg;
pub use sniffer_plugin as plugin;
pub use sniffer_render as render;

#[cfg(target_os = "android")]
pub use sniffer_platform_android as platform;

#[cfg(not(target_os = "android"))]
pub use sniffer_platform_desktop as platform;

/// Common engine types and traits for building embedded UI applications.
pub mod prelude {
    pub use sniffer_core::anim::{SpringConfig, SpringSimulation, TransitionManager};
    pub use sniffer_core::layout::{LayoutNode, calculate_layout};
    pub use sniffer_core::math::{EdgeInsets, Point, Rect, ScreenMetrics, Size};
    pub use sniffer_core::physics::{
        ScrollPhysics, sync_max_scroll_from_layout, sync_scroll_physics_from_tree,
    };
    pub use sniffer_core::style::{Easing, FlexDirection, JustifyContent, Style, Transform};
    pub use sniffer_core::types::{Action, DismissDirection, Element, ScrollDismissConfig};
    pub use sniffer_core::ui::event::{EventBus, UiEvent};
    pub use sniffer_core::ui::widget::{WidgetId, fnv1a};
    pub use sniffer_plugin::{JsPlugin, PluginLoader, PluginRegistry, UiPlugin};
}

/// Launch the SnifferEngine desktop window loop.
#[cfg(not(target_os = "android"))]
pub fn run_desktop() -> Result<(), Box<dyn std::error::Error>> {
    sniffer_platform_desktop::run()
}

/// Launch the SnifferEngine Android NativeActivity loop.
#[cfg(target_os = "android")]
pub fn run_android(app: android_activity::AndroidApp) {
    sniffer_platform_android::android_main(app);
}

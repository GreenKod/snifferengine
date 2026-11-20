use sniffer_core::ui::data_map::DataMap;
use sniffer_core::ui::event::UiEvent;
use sniffer_core::ui::style_map::StyleMap;
use sniffer_core::ui::widget::WidgetId;

/// Sentinel subscription that receives every widget-scoped event regardless of target.
///
/// Most plugins subscribe only to the widget IDs they render. Plugins that perform
/// their own dynamic routing (for example JavaScript plugins whose `onEvent` entry
/// point resolves IDs at runtime) subscribe to this sentinel instead.
pub const WILDCARD_WIDGET_ID: WidgetId = WidgetId::MAX;

/// Interface that external plugins must implement.
///
/// When a plugin is registered, the system reads `subscriptions()` and
/// inserts `ID -> [this_plugin]` entries into `PluginRegistry`'s internal `HashMap`.
/// When a widget-scoped event arrives, only the listeners for that specific ID
/// (plus [`WILDCARD_WIDGET_ID`] subscribers) are invoked. Global events that are not
/// scoped to a widget are delivered to every registered plugin.
///
/// # Thread Safety
/// `Send + Sync` is required; plugins are called from the render thread
/// but may be shared across other threads.
pub trait UiPlugin: Send + Sync {
    /// The widget IDs this plugin wants to observe.
    ///
    /// Returns a slice — no heap allocation, zero cost. Include
    /// [`WILDCARD_WIDGET_ID`] to receive every widget-scoped event.
    fn subscriptions(&self) -> &[WidgetId];

    /// Called when a relevant `UiEvent` is received for a subscribed ID.
    ///
    /// The plugin may update `StyleMap` or `DataMap` to change the appearance
    /// or content of widgets without touching `app.rs`.
    fn on_event(
        &self,
        event: &UiEvent,
        styles: &StyleMap,
        data: &DataMap,
        actions: &std::sync::Arc<std::sync::Mutex<Vec<sniffer_core::types::Action>>>,
    );

    /// Ask the plugin to provide the initial UI layout tree.
    ///
    /// If multiple plugins provide a layout, the system may use the first one or merge them.
    /// Default implementation returns `None`, indicating the plugin doesn't define layout.
    fn build_ui(&self) -> Option<sniffer_core::types::Element> {
        None
    }

    /// Called once per render frame.
    /// Useful for periodic tasks, animations, or garbage collection.
    fn on_tick(&self) {}

    /// Returns `true` if this plugin currently has active timers registered.
    /// Used by the host registry to avoid sending unnecessary tick messages.
    fn has_active_timers(&self) -> bool {
        true
    }

    /// Called when another plugin broadcasts a message to all plugins via `broadcastEvent`.
    ///
    /// # Arguments
    /// * `channel` — The broadcast channel name (e.g. `"store.changed"`).
    /// * `payload_json` — A JSON string containing the broadcast data.
    fn on_broadcast(&self, _channel: &str, _payload_json: &str) {}

    /// Called when the plugin is moved out of viewport / idle to suspend background ticks.
    fn on_suspend(&self) {}

    /// Called when the plugin becomes active again.
    fn on_resume(&self) {}

    /// Called when the plugin is being unloaded.
    fn on_unload(&self) {}
}

#![cfg(not(target_os = "android"))]

use sniffer_core::types::Action;
use sniffer_core::ui::data_map::DataMap;
use sniffer_core::ui::event::{EventBus, UiEvent};
use sniffer_core::ui::style_map::StyleMap;
use sniffer_core::ui::widget::fnv1a;
use sniffer_plugin::js::JsPluginConfig;
use sniffer_plugin::{JsPlugin, PluginRegistry, UiPlugin};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Script whose `onEvent` uninstalls its own global binding after the first call.
///
/// If the worker resolved `onEvent` by name on every event, the second event would be
/// dropped. With a cached `Persistent` reference both events reach the handler, which
/// is exactly what this test asserts.
const SELF_UNINSTALLING_HANDLER: &str = r#"
    globalThis.__count = 0;
    globalThis.onEvent = function (_eventJson) {
        globalThis.__count += 1;
        if (globalThis.__count === 1) {
            globalThis.onEvent = null;
        }
        host_set_ui(JSON.stringify({
            Label: { id: "event_counter", text: String(globalThis.__count), style: {} }
        }));
        return "[]";
    };
"#;

fn dispatch(registry: &PluginRegistry, bus: &EventBus) {
    let actions = Arc::new(Mutex::new(Vec::new()));
    registry.dispatch(bus, &StyleMap::default(), &DataMap::default(), &actions);
}

fn current_label_text(plugin: &JsPlugin) -> Option<String> {
    match UiPlugin::build_ui(plugin)? {
        sniffer_core::types::Element::Label { text, .. } => Some(text),
        _ => None,
    }
}

fn wait_for_label(plugin: &JsPlugin, expected: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if current_label_text(plugin).as_deref() == Some(expected) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn on_event_handler_reference_is_cached_across_events() {
    let mut registry = PluginRegistry::default();
    let actions = Arc::new(Mutex::new(Vec::<Action>::new()));

    let plugin = Arc::new(
        JsPlugin::new(JsPluginConfig {
            script_content: SELF_UNINSTALLING_HANDLER.to_string(),
            plugin_id: "com.example.cached_handler".to_string(),
            is_master: true,
            vault: registry.vault(),
            action_queue: actions,
            api_map: registry.api_registry(),
            broadcast_queue: registry.broadcast_queue(),
            permissions: vec!["plugin.permission.UI".to_string()],
            default_settings: serde_json::Value::Null,
            cached_ui: None,
            cache_path: None,
            pkg_registry: None,
            max_memory_mb: Some(8),
        })
        .expect("plugin should initialize"),
    );
    registry.register(&(Arc::clone(&plugin) as Arc<dyn UiPlugin>));

    let bus = EventBus::default();
    bus.push(UiEvent::Click(fnv1a(b"first_target"), 1.0, 1.0));
    dispatch(&registry, &bus);
    assert!(
        wait_for_label(&plugin, "1"),
        "first event must reach onEvent"
    );

    // `onEvent` was uninstalled by the handler itself; the cached reference must
    // still deliver the next event.
    bus.push(UiEvent::Click(fnv1a(b"second_target"), 2.0, 2.0));
    dispatch(&registry, &bus);
    assert!(
        wait_for_label(&plugin, "2"),
        "cached onEvent reference must keep receiving events after the global binding is removed"
    );

    plugin.on_unload();
}

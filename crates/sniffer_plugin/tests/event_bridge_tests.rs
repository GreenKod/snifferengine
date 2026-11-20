#![cfg(not(target_os = "android"))]

//! Verifies the Fast Event Bridge contract:
//!
//! * handlers that keep the framework dispatcher receive structured objects,
//! * custom handlers keep receiving JSON strings (backward compatibility),
//! * plugins can opt a custom handler into the object bridge explicitly.

use sniffer_core::types::Action;
use sniffer_core::ui::data_map::DataMap;
use sniffer_core::ui::event::{EventBus, UiEvent};
use sniffer_core::ui::style_map::StyleMap;
use sniffer_core::ui::widget::fnv1a;
use sniffer_plugin::js::JsPluginConfig;
use sniffer_plugin::{JsPlugin, PluginRegistry, UiPlugin};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Writes the observed payload kind and event type into a label so the test can read
/// it back without a custom test host.
const PROBE_BODY: &str = r#"
    host_set_ui(JSON.stringify({
        Label: {
            id: "bridge_probe",
            text: typeof payload + ":" + event.type,
            style: {}
        }
    }));
    return "[]";
"#;

fn probe_script(payload_kind: &str) -> String {
    match payload_kind {
        "framework" => format!(
            r"
            globalThis.onEvent = function (payload) {{
                var event = payload;
                {PROBE_BODY}
            }};
            globalThis.__snifferDefaultOnEvent = globalThis.onEvent;
            "
        ),
        "opt-in" => format!(
            r#"
            globalThis.onEvent = function (payload) {{
                var event = payload;
                {PROBE_BODY}
            }};
            globalThis.__snifferEventFormat = "object";
            "#
        ),
        _ => format!(
            r"
            globalThis.onEvent = function (payload) {{
                var event = JSON.parse(payload);
                {PROBE_BODY}
            }};
            "
        ),
    }
}

fn dispatch_event(script: &str, plugin_id: &str, event: UiEvent) -> Option<String> {
    let mut registry = PluginRegistry::default();
    let actions = Arc::new(Mutex::new(Vec::<Action>::new()));

    let plugin = Arc::new(
        JsPlugin::new(JsPluginConfig {
            script_content: script.to_string(),
            plugin_id: plugin_id.to_string(),
            is_master: true,
            vault: registry.vault(),
            action_queue: Arc::clone(&actions),
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
    bus.push(event);
    registry.dispatch(&bus, &StyleMap::default(), &DataMap::default(), &actions);

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(sniffer_core::types::Element::Label { text, .. }) =
            UiPlugin::build_ui(plugin.as_ref())
        {
            plugin.on_unload();
            return Some(text);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    plugin.on_unload();
    None
}

#[test]
fn framework_dispatcher_receives_structured_event_objects() {
    let text = dispatch_event(
        &probe_script("framework"),
        "com.example.bridge_framework",
        UiEvent::Click(fnv1a(b"bridge_target"), 1.0, 1.0),
    );
    assert_eq!(
        text.as_deref(),
        Some("object:Click"),
        "the framework dispatcher must receive a ready-made QuickJS object"
    );
}

#[test]
fn custom_handler_keeps_receiving_json_strings() {
    let text = dispatch_event(
        &probe_script("legacy"),
        "com.example.bridge_legacy",
        UiEvent::Click(fnv1a(b"bridge_target"), 1.0, 1.0),
    );
    assert_eq!(
        text.as_deref(),
        Some("string:Click"),
        "plugins that install their own handler must keep the legacy JSON string contract"
    );
}

#[test]
fn custom_handler_can_opt_into_the_object_bridge() {
    let text = dispatch_event(
        &probe_script("opt-in"),
        "com.example.bridge_opt_in",
        UiEvent::Scroll(Some(fnv1a(b"scroll_target")), 0.0, 32.0, 0.0, 640.0),
    );
    assert_eq!(
        text.as_deref(),
        Some("object:Scroll"),
        "explicitly opted-in handlers must receive structured objects"
    );
}

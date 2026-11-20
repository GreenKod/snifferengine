#![cfg(not(target_os = "android"))]

//! Regression tests for the debounced plugin UI cache writer.
//!
//! `host_set_ui` used to run `postcard` serialization and enqueue a disk write on
//! every call. The writer now waits for a quiet period and snapshots the shared
//! tree once, so:
//!
//! * no cache file appears while updates keep arriving within the debounce window,
//! * the persisted state is always the latest one, and
//! * plugin unload flushes the pending write immediately instead of losing it.

use rquickjs::{Context, Runtime};
use sniffer_core::types::Element;
use sniffer_plugin::js::bindings_ui::register_ui_bindings;
use sniffer_plugin::js::{JsPlugin, JsPluginConfig};
use sniffer_plugin::{PluginRegistry, UiPlugin};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type SharedUiTree = Arc<Mutex<Option<Arc<Element>>>>;

fn unique_temp_path(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sniffer_test_cache_debounce_{label}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join("plugin.ui.bin")
}

fn set_ui_script(text: &str) -> String {
    format!(
        r#"host_set_ui(JSON.stringify({{
            Container: {{
                id: "root",
                style: {{}},
                children: [{{ Label: {{ id: "label", text: "{text}", style: {{}} }} }}]
            }}
        }}));"#
    )
}

fn cached_label_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let tree = postcard::from_bytes::<Element>(&bytes).ok()?;
    match tree {
        Element::Container { children, .. } => match children.first() {
            Some(Element::Label { text, .. }) => Some(text.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn wait_for_cache_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn register_bindings(cache_path: &Path, steps: &[String]) -> SharedUiTree {
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ui_tree: SharedUiTree = Arc::new(Mutex::new(None));
    let permissions = vec!["plugin.permission.UI".to_string()];
    let granted = Arc::new(Mutex::new(permissions.clone()));

    context.with(|ctx| {
        register_ui_bindings(
            &ctx,
            &ctx.globals(),
            Arc::clone(&ui_tree),
            &permissions,
            granted,
            Some(cache_path.to_path_buf()),
        );
        for step in steps {
            ctx.eval::<(), _>(step.as_str()).expect("script step");
        }
    });

    ui_tree
}

#[test]
fn rapid_ui_updates_collapse_into_one_debounced_write() {
    let cache_path = unique_temp_path("burst");

    let burst: Vec<String> = ["v1", "v2", "v3", "v4", "v5"]
        .iter()
        .map(|text| set_ui_script(text))
        .collect();
    register_bindings(&cache_path, &burst);

    // The last update just happened, so the debounce window is still open: the old
    // immediate writer would have produced a file by now.
    assert!(
        !cache_path.exists(),
        "no cache write may happen while the debounce window is still open"
    );

    assert!(
        wait_for_cache_file(&cache_path, Duration::from_secs(2)),
        "the debounced writer must eventually persist the tree"
    );
    assert_eq!(
        cached_label_text(&cache_path).as_deref(),
        Some("v5"),
        "the single debounced write must persist the latest state, not an intermediate one"
    );
}

#[test]
fn plugin_unload_flushes_the_pending_cache_write() {
    let cache_path = unique_temp_path("unload");
    let mut registry = PluginRegistry::default();
    let actions = Arc::new(Mutex::new(Vec::new()));

    let plugin = Arc::new(
        JsPlugin::new(JsPluginConfig {
            script_content: set_ui_script("final_state"),
            plugin_id: "com.example.cache_flush".to_string(),
            is_master: true,
            vault: registry.vault(),
            action_queue: Arc::clone(&actions),
            api_map: registry.api_registry(),
            broadcast_queue: registry.broadcast_queue(),
            permissions: vec!["plugin.permission.UI".to_string()],
            default_settings: serde_json::Value::Null,
            cached_ui: None,
            cache_path: Some(cache_path.clone()),
            pkg_registry: None,
            max_memory_mb: Some(8),
        })
        .expect("plugin should initialize"),
    );
    registry.register(&(Arc::clone(&plugin) as Arc<dyn UiPlugin>));

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(Element::Container { .. }) = UiPlugin::build_ui(plugin.as_ref()).as_ref() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    plugin.on_unload();
    // The deadline is well below the debounce period: only the explicit unload
    // flush can produce the file this fast.
    assert!(
        wait_for_cache_file(&cache_path, Duration::from_millis(200)),
        "unload must flush the debounced write immediately"
    );
    assert_eq!(
        cached_label_text(&cache_path).as_deref(),
        Some("final_state"),
        "the flushed write must contain the last UI state"
    );
}

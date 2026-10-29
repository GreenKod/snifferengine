#![cfg(not(target_os = "android"))]

use std::fs;
use std::path::{Path, PathBuf};

fn get_framework_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../framework")
        .canonicalize()
        .expect("Could not find framework directory")
}

#[test]
fn test_framework_javascript_syntax() {
    let framework_dir = get_framework_dir();
    let rt = rquickjs::Runtime::new().expect("Failed to create QuickJS runtime");
    let ctx = rquickjs::Context::full(&rt).expect("Failed to create QuickJS context");

    let mut js_files = Vec::new();
    if let Ok(entries) = fs::read_dir(&framework_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("js") {
                js_files.push(path);
            }
        }
    }

    assert!(
        !js_files.is_empty(),
        "No JS files found in framework directory"
    );

    for file in js_files {
        let code =
            fs::read_to_string(&file).unwrap_or_else(|e| panic!("Failed to read {file:?}: {e}"));

        ctx.with(|c| {
            let res = c.eval::<rquickjs::Value, _>(code.as_bytes());
            if let Err(e) = res {
                let caught = c.catch();
                let exc = caught.as_exception();
                let msg = exc.as_ref().and_then(|x| x.message()).unwrap_or_default();
                assert!(
                    !msg.contains("SyntaxError"),
                    "Syntax error in JavaScript file {file:?}: {msg}\n{e}"
                );
            }
        });
    }
}

#[test]
fn test_plugin_loader_rejects_tampered_checksum() {
    let temp_dir = std::env::temp_dir().join(format!("sniffer_test_tamper_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("tampered_plugin")).expect("Failed to create temp dir");

    let plugins_json = r#"{ "active_plugins": ["tampered_plugin"] }"#;
    fs::write(temp_dir.join("plugins.json"), plugins_json).expect("Failed to write plugins.json");

    let manifest_json = r#"{
        "id": "com.sniffer.tampered",
        "name": "Tampered Plugin",
        "version": "1.0.0",
        "main": "main.js",
        "permissions": ["plugin.permission.UI"],
        "checksums": {
            "main.js": "0000000000000000000000000000000000000000000000000000000000000000"
        }
    }"#;
    fs::write(
        temp_dir.join("tampered_plugin/manifest.json"),
        manifest_json,
    )
    .expect("Failed to write manifest.json");

    let js_code = "const x = 123;";
    fs::write(temp_dir.join("tampered_plugin/main.js"), js_code).expect("Failed to write main.js");

    let mut registry = sniffer_plugin::PluginRegistry::default();
    let action_queue = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let loader = sniffer_plugin::loader::PluginLoader::new(&temp_dir);
    loader.register_all(&mut registry, &action_queue);

    // Plugin MUST NOT be registered due to checksum mismatch
    assert_eq!(
        registry.len(),
        0,
        "Tampered plugin should have been rejected by the loader"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_plugin_loader_normalizes_crlf_checksum() {
    let temp_dir = std::env::temp_dir().join(format!("sniffer_test_crlf_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("crlf_plugin")).expect("Failed to create temp dir");

    let plugins_json = r#"{ "active_plugins": ["crlf_plugin"] }"#;
    fs::write(temp_dir.join("plugins.json"), plugins_json).expect("Failed to write plugins.json");

    // The code with LF has a known hash
    let lf_code = "const x = 1;\nconst y = 2;\n";
    let lf_hash = sniffer_pkg::manifest::sha256::compute_sha256_hex(lf_code.as_bytes());

    let manifest_json = format!(
        r#"{{
        "id": "com.sniffer.crlf",
        "name": "CRLF Plugin",
        "version": "1.0.0",
        "main": "main.js",
        "permissions": ["plugin.permission.UI"],
        "checksums": {{
            "main.js": "{lf_hash}"
        }}
    }}"#
    );
    fs::write(temp_dir.join("crlf_plugin/manifest.json"), manifest_json)
        .expect("Failed to write manifest.json");

    // Write file with Windows CRLF (\r\n) line endings on disk
    let crlf_code = "const x = 1;\r\nconst y = 2;\r\n";
    fs::write(temp_dir.join("crlf_plugin/main.js"), crlf_code).expect("Failed to write main.js");

    let mut registry = sniffer_plugin::PluginRegistry::default();
    let action_queue = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let loader = sniffer_plugin::loader::PluginLoader::new(&temp_dir);
    loader.register_all(&mut registry, &action_queue);

    // Plugin MUST be successfully registered because CRLF was normalized to LF!
    assert_eq!(
        registry.len(),
        1,
        "Plugin with CRLF line endings should be accepted after normalization"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_generic_framework_declarative_scrollview_and_event_routing_phase_3() {
    let framework_dir = get_framework_dir();
    let framework_path = framework_dir.join("sniffer_ui.js");

    let rt = rquickjs::Runtime::new().unwrap();
    let ctx = rquickjs::Context::full(&rt).unwrap();

    ctx.with(|c| {
        let setup = r"
            globalThis.host_log = function() {};
            globalThis.host_set_ui = function() {};
            globalThis.host_screen_width = function() { return 1080; };
            globalThis.host_screen_height = function() { return 2400; };
            globalThis.host_hash = function(s) {
                let h = 0x811c9dc5;
                for (let i = 0; i < s.length; i++) {
                    h ^= s.charCodeAt(i);
                    h = Math.imul(h, 0x01000193);
                }
                return (h >>> 0);
            };
        ";
        c.eval::<(), _>(setup).unwrap();

        let fw_code = fs::read_to_string(&framework_path).unwrap();
        c.eval::<(), _>(fw_code).unwrap();

        // 1. Test declarative single-object ScrollView with threshold_dp normalization
        let sv_json: String = c
            .eval(
                r#"
                JSON.stringify(ScrollView({
                    id: "gallery_sheet_scroll",
                    rubber_band: 0.55,
                    dismiss_config: {
                        direction: "Down",
                        threshold_dp: 75,
                        target_container_id: "gallery_sheet_modal",
                        action_event: "gallery_sheet_dismiss"
                    },
                    children: [
                        Label("sheet_title", "Photo Gallery", {})
                    ]
                }))
                "#,
            )
            .unwrap();

        let sv_el: sniffer_core::types::Element = serde_json::from_str(&sv_json).unwrap();
        match sv_el {
            sniffer_core::types::Element::ScrollView {
                id,
                rubber_band,
                dismiss_config,
                children,
                ..
            } => {
                assert_eq!(id.as_deref(), Some("gallery_sheet_scroll"));
                assert_eq!(rubber_band, Some(0.55));
                let cfg = dismiss_config.expect("dismiss_config must be deserialized");
                assert_eq!(cfg.direction, sniffer_core::types::DismissDirection::Down);
                assert_eq!(cfg.threshold, Some(75.0));
                assert_eq!(
                    cfg.target_container_id.as_deref(),
                    Some("gallery_sheet_modal")
                );
                assert_eq!(cfg.action_event.as_deref(), Some("gallery_sheet_dismiss"));
                assert_eq!(children.len(), 1);
            }
            other => panic!("Expected ScrollView, got {other:?}"),
        }

        // 2. Test SnifferUI.onBack and SnifferUI.onDismiss event routing in a standalone app
        c.eval::<(), _>(
            r#"
            globalThis.sheetState = {
                isOpen: true,
                dismissCount: 0,
                backCount: 0
            };
            SnifferUI.onBack(function() {
                if (sheetState.isOpen) {
                    sheetState.isOpen = false;
                    sheetState.backCount += 1;
                    return true;
                }
                return false;
            });
            SnifferUI.onDismiss("gallery_sheet_dismiss", function(e) {
                sheetState.isOpen = false;
                sheetState.dismissCount += 1;
                return true;
            });
            "#,
        )
        .unwrap();

        let handled_back: bool = c
            .eval(r#"SnifferUI.dispatchEvent({ type: "Back" })"#)
            .unwrap();
        assert!(handled_back, "First Back event when open must be handled");
        let is_open: bool = c.eval("sheetState.isOpen").unwrap();
        assert!(!is_open);

        let handled_back_when_closed: bool = c
            .eval(r#"SnifferUI.dispatchEvent({ type: "Back" })"#)
            .unwrap();
        assert!(
            !handled_back_when_closed,
            "Second Back event when closed must return false"
        );

        let handled_dismiss: bool = c
            .eval(
                r#"SnifferUI.dispatchEvent(JSON.stringify({
                    type: "ScrollDismiss",
                    widget_id: 12345,
                    displacement: 180.0,
                    velocity: -600.0,
                    action: "gallery_sheet_dismiss"
                }))"#,
            )
            .unwrap();
        assert!(handled_dismiss, "ScrollDismiss action must be routed");
        let dismiss_count: i32 = c.eval("sheetState.dismissCount").unwrap();
        assert_eq!(dismiss_count, 1);
    });
}

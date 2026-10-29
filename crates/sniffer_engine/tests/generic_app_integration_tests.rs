#![cfg(not(target_os = "android"))]

use sniffer_engine::core::ui::data_map::DataMap;
use sniffer_engine::core::ui::style_map::StyleMap;
use sniffer_engine::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn get_framework_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../framework")
        .canonicalize()
        .expect("Could not find framework directory")
}

fn find_element<'a>(el: &'a Element, target_id: &str) -> Option<&'a Element> {
    if el.id() == Some(target_id) {
        return Some(el);
    }
    match el {
        Element::Container { children, .. }
        | Element::ScrollView { children, .. }
        | Element::SharedView { children, .. } => {
            for child in children {
                if let Some(found) = find_element(child, target_id) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

#[test]
fn test_standalone_non_launcher_app_elastic_scroll_and_dismiss_via_sniffer_engine() {
    let temp_dir = std::env::temp_dir().join(format!(
        "sniffer_engine_standalone_app_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("framework")).unwrap();
    fs::create_dir_all(temp_dir.join("settings_sheet")).unwrap();

    // Copy generic SnifferUI framework into temporary workspace
    let fw_src = get_framework_dir().join("sniffer_ui.js");
    fs::copy(&fw_src, temp_dir.join("framework/sniffer_ui.js")).unwrap();

    let plugins_json = r#"{
        "active_plugins": ["settings_sheet"],
        "master_plugin": "settings_sheet"
    }"#;
    fs::write(temp_dir.join("plugins.json"), plugins_json).unwrap();

    let manifest_json = r#"{
        "id": "com.example.settings_sheet",
        "name": "Standalone Settings & Media Sheet",
        "version": "1.0.0",
        "main": "main.js",
        "preload": ["../framework/sniffer_ui.js"],
        "permissions": ["plugin.permission.UI"]
    }"#;
    fs::write(temp_dir.join("settings_sheet/manifest.json"), manifest_json).unwrap();

    // A standalone non-launcher application (Settings & Media Sheet) with zero launcher concepts
    let main_js = r#"
        var appState = {
            sheetOpen: true,
            lastDismissVelocity: 0
        };

        function renderApp() {
            var children = [
                Container("header_bar", { width: 1080, height: 80 }, [
                    Label("header_title", "System Settings", { text_size: 24 })
                ])
            ];

            if (appState.sheetOpen) {
                var items = [];
                for (var i = 0; i < 20; i++) {
                    items.push(
                        Container("setting_row_" + i, { width: 1000, height: 120 }, [
                            Label("setting_label_" + i, "Option #" + i, { text_size: 18 })
                        ])
                    );
                }

                children.push(
                    Container("settings_sheet_modal", {
                        width: 1080,
                        height: 1800,
                        opacity: 1.0,
                        transform: { translate_x: 0, translate_y: 0, scale: 1.0, rotate: 0 },
                        transition: {
                            duration: 0.35,
                            delay: 0.0,
                            easing: { spring: { stiffness: 260.0, damping: 24.0 } }
                        }
                    }, [
                        ScrollView({
                            id: "settings_sheet_scroll",
                            rubber_band: 0.55,
                            dismiss_config: {
                                direction: "Down",
                                threshold_dp: 60,
                                target_container_id: "settings_sheet_modal",
                                action_event: "settings_sheet_dismiss"
                            },
                            style: { width: 1080, height: 1600, flex_direction: "Column" },
                            children: items
                        })
                    ])
                );
            }

            host_set_ui(JSON.stringify(
                Container("standalone_app_root", {
                    width: 1080,
                    height: 2400,
                    flex_direction: "Column"
                }, children)
            ));
        }

        SnifferUI.onDismiss("settings_sheet_dismiss", function(e) {
            appState.sheetOpen = false;
            appState.lastDismissVelocity = e.velocity || 0;
            renderApp();
            return true;
        });

        SnifferUI.onBack(function() {
            if (appState.sheetOpen) {
                appState.sheetOpen = false;
                renderApp();
                return true;
            }
            return false;
        });

        SnifferUI.on("Click", function(e) {
            if (SnifferUI.matchId(e.id, "header_bar")) {
                appState.sheetOpen = true;
                renderApp();
                return true;
            }
            return false;
        });

        globalThis.onEvent = function(eventJson) {
            SnifferUI.dispatchEvent(eventJson);
            return "[]";
        };

        renderApp();
    "#;
    fs::write(temp_dir.join("settings_sheet/main.js"), main_js).unwrap();

    // 1. Load standalone plugin into SnifferEngine's PluginRegistry
    let mut registry = PluginRegistry::default();
    let actions = Arc::new(Mutex::new(Vec::new()));
    let loader = PluginLoader::new(&temp_dir);
    loader.register_all(&mut registry, &actions);
    assert_eq!(registry.len(), 1, "Standalone plugin must be registered");

    // Wait briefly for initial async QuickJS evaluation to publish UI tree
    let start = Instant::now();
    let mut ui_tree = None;
    while start.elapsed() < Duration::from_secs(2) {
        if let Some(tree) = registry.build_ui() {
            ui_tree = Some(tree);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let tree = ui_tree.expect("Standalone plugin must publish initial UI tree");
    assert!(
        find_element(&tree, "settings_sheet_modal").is_some(),
        "Modal sheet must be initially mounted"
    );

    // 2. Run SnifferEngine Layout, TransitionManager, and ScrollPhysics sync
    let layout = calculate_layout(&tree, Size::new(1080.0, 2400.0), 0.0, 0.0);
    let mut transition_manager = TransitionManager::default();
    transition_manager.sync_tree(&tree);

    let mut scroll_physics: HashMap<u64, ScrollPhysics> = HashMap::new();
    sync_scroll_physics_from_tree(&tree, &mut scroll_physics);
    sync_max_scroll_from_layout(&tree, &layout, &mut scroll_physics);

    let scroll_wid = fnv1a(b"settings_sheet_scroll");
    let phys = scroll_physics
        .get_mut(&scroll_wid)
        .expect("ScrollPhysics must be synced for settings_sheet_scroll");

    assert_eq!(phys.rubber_band, Some(0.55));
    let dismiss_cfg = phys
        .dismiss_config
        .as_ref()
        .expect("ScrollDismissConfig must be synced");
    assert_eq!(dismiss_cfg.direction, DismissDirection::Down);
    assert_eq!(dismiss_cfg.threshold, Some(60.0));
    assert_eq!(
        dismiss_cfg.target_container_id.as_deref(),
        Some("settings_sheet_modal")
    );
    assert_eq!(
        dismiss_cfg.action_event.as_deref(),
        Some("settings_sheet_dismiss")
    );

    // 3. Test sub-threshold elastic overscroll and spring recovery (no dismiss)
    let ratio = phys.rubber_band.unwrap();
    phys.apply_drag_y(-40.0 * ratio);
    assert!(
        phys.pos_y < 0.0,
        "Pulling down at top of list must create negative overscroll"
    );
    phys.release_drag_y(0.0);
    for _ in 0..120 {
        phys.tick(1.0 / 120.0);
    }
    assert!(
        phys.pos_y.abs() < 1.0,
        "Sub-threshold overscroll must spring back to 0.0 (got {})",
        phys.pos_y
    );

    // 4. Test super-threshold pull-to-dismiss handoff & generic event routing
    let phys = scroll_physics.get_mut(&scroll_wid).unwrap();
    phys.apply_drag_y(-220.0 * ratio);
    let pulled_displacement = (-phys.pos_y).max(0.0);
    assert!(
        pulled_displacement > 60.0,
        "Pulled displacement ({pulled_displacement}) must exceed 60dp threshold"
    );

    // Transfer displacement to target container in TransitionManager (zero-copy &str lookup)
    let cfg = phys.dismiss_config.as_ref().unwrap();
    if let Some(target_id) = cfg.target_container_id.as_deref()
        && let Some(anim) = transition_manager.states.get_mut(target_id)
    {
        anim.current_style.transform.translate_y = pulled_displacement;
    }
    let action_name = cfg.action_event.clone();
    phys.pos_y = 0.0;

    assert_eq!(
        transition_manager
            .get_current_style("settings_sheet_modal")
            .unwrap()
            .transform
            .translate_y,
        pulled_displacement
    );

    // Emit generic UiEvent::ScrollDismiss into EventBus and dispatch to plugin
    let bus = EventBus::default();
    let styles = StyleMap::default();
    let data = DataMap::default();
    bus.push(UiEvent::ScrollDismiss {
        widget_id: scroll_wid,
        displacement: pulled_displacement,
        velocity: -750.0,
        action: action_name,
    });
    registry.dispatch(&bus, &styles, &data, &actions);

    // Wait for QuickJS plugin to process ScrollDismiss and update UI tree
    let wait_dismiss = Instant::now();
    let mut dismissed_ok = false;
    while wait_dismiss.elapsed() < Duration::from_secs(2) {
        if let Some(updated_tree) = registry.build_ui()
            && find_element(&updated_tree, "settings_sheet_modal").is_none()
        {
            dismissed_ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        dismissed_ok,
        "Standalone plugin must unmount settings_sheet_modal upon UiEvent::ScrollDismiss"
    );

    // 5. Re-open via Click on header_bar, then dismiss via generic UiEvent::Back
    bus.push(UiEvent::Click(fnv1a(b"header_bar"), 100.0, 40.0));
    registry.dispatch(&bus, &styles, &data, &actions);

    let wait_reopen = Instant::now();
    let mut reopened_ok = false;
    while wait_reopen.elapsed() < Duration::from_secs(2) {
        if let Some(updated_tree) = registry.build_ui()
            && find_element(&updated_tree, "settings_sheet_modal").is_some()
        {
            reopened_ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(reopened_ok, "Clicking header_bar must reopen modal sheet");

    bus.push(UiEvent::Back);
    registry.dispatch(&bus, &styles, &data, &actions);

    let wait_back = Instant::now();
    let mut back_closed_ok = false;
    while wait_back.elapsed() < Duration::from_secs(2) {
        if let Some(updated_tree) = registry.build_ui()
            && find_element(&updated_tree, "settings_sheet_modal").is_none()
        {
            back_closed_ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        back_closed_ok,
        "Generic UiEvent::Back must be routed to SnifferUI.onBack and close modal sheet"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

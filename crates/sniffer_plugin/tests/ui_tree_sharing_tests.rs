#![cfg(not(target_os = "android"))]

//! Regression tests for the shared (`Arc<Element>`) plugin UI tree.
//!
//! `host_set_ui` used to deep-clone the freshly parsed tree into the mutation buffer,
//! then move another copy into the tree. The tree is now shared through an `Arc`, so
//! these tests pin down the two properties that make the sharing safe:
//!
//! * mutations issued right after `host_set_ui` still reach the new tree, and
//! * a snapshot taken by a reader is not corrupted by later copy-on-write updates.

use rquickjs::{Context, Runtime};
use sniffer_core::types::Element;
use sniffer_plugin::js::bindings_ui::register_ui_bindings;
use std::sync::{Arc, Mutex};

type SharedTree = Arc<Mutex<Option<Arc<Element>>>>;

fn with_ui_tree(script_steps: &[&str]) -> SharedTree {
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ui_tree: SharedTree = Arc::new(Mutex::new(None));
    let permissions = vec!["plugin.permission.UI".to_string()];
    let granted = Arc::new(Mutex::new(permissions.clone()));

    context.with(|ctx| {
        register_ui_bindings(
            &ctx,
            &ctx.globals(),
            Arc::clone(&ui_tree),
            &permissions,
            granted,
            None,
        );
        for step in script_steps {
            ctx.eval::<(), _>(*step).expect("script step");
        }
    });

    ui_tree
}

fn root_text_size(tree: &SharedTree) -> Option<f32> {
    let guard = tree.lock().unwrap();
    let root = guard.as_ref()?;
    match root.as_ref() {
        Element::Container { style, .. } => Some(style.text_size),
        _ => None,
    }
}

fn set_ui(text_size: f32) -> String {
    format!(
        r#"host_set_ui(JSON.stringify({{
            Container: {{
                id: "root",
                style: {{ text_size: {text_size} }},
                children: [{{ Label: {{ id: "child", text: "hello", style: {{}} }} }}]
            }}
        }}));"#
    )
}

#[test]
fn mutations_after_set_ui_reach_the_shared_tree() {
    let tree = with_ui_tree(&[
        &set_ui(12.0),
        r#"host_update_style("root", "text_size", "20");"#,
    ]);

    assert_eq!(
        root_text_size(&tree),
        Some(20.0),
        "a style mutation issued right after host_set_ui must apply to the new tree"
    );
    assert_eq!(
        tree.lock().unwrap().as_ref().map(Arc::strong_count),
        Some(1),
        "the tree must not stay aliased after the mutation buffer is flushed"
    );
}

#[test]
fn snapshots_survive_copy_on_write_updates() {
    let tree = with_ui_tree(&[&set_ui(12.0)]);

    let snapshot = {
        let guard = tree.lock().unwrap();
        Arc::clone(guard.as_ref().expect("tree must be populated"))
    };
    assert_eq!(root_text_size(&tree), Some(12.0));

    // A reader holding a snapshot forces the writer onto a fresh allocation; the tree
    // itself must still pick up the new value.
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let permissions = vec!["plugin.permission.UI".to_string()];
    let granted = Arc::new(Mutex::new(permissions.clone()));
    context.with(|ctx| {
        register_ui_bindings(
            &ctx,
            &ctx.globals(),
            Arc::clone(&tree),
            &permissions,
            granted,
            None,
        );
        ctx.eval::<(), _>(r#"host_update_style("root", "text_size", "28");"#)
            .expect("style mutation");
    });

    assert_eq!(
        root_text_size(&tree),
        Some(28.0),
        "the live tree must receive the mutation"
    );
    match snapshot.as_ref() {
        Element::Container { style, .. } => assert_eq!(
            style.text_size, 12.0,
            "an existing snapshot must not be mutated behind the reader's back"
        ),
        other => panic!("unexpected snapshot root: {other:?}"),
    }
}

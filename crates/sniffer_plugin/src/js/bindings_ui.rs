use crate::js::permission_manager::permission_granted;
use crate::{dev_err, dev_log};
use obfstr::obfstr;
use rquickjs::{Ctx, Function, Object, Value};
use sniffer_core::types::Element;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Shared handle to a plugin's UI tree.
type SharedUiTree = Arc<Mutex<Option<Arc<Element>>>>;

/// Minimum quiet period before the cache writer persists a plugin tree.
///
/// Rapid `host_set_ui` bursts (e.g. animated state changes) collapse into a single
/// write that snapshots the latest tree state, so neither `postcard` serialization
/// nor a disk write runs on every update.
const CACHE_WRITE_DEBOUNCE: Duration = Duration::from_millis(300);

/// Upper bound for the idle wait when no cache write is pending.
const CACHE_WRITER_IDLE_WAIT: Duration = Duration::from_secs(3600);

enum CacheWriterMsg {
    /// (Re)arms the debounce timer for a cache path and remembers where to read the
    /// latest tree state from when the timer expires.
    Schedule { path: PathBuf, tree: SharedUiTree },
    /// Persists the pending state for a path immediately (plugin unload) and drops
    /// it from the schedule so the tree handle can be released.
    FlushNow { path: PathBuf },
}

static CACHE_WRITER_TX: OnceLock<SyncSender<CacheWriterMsg>> = OnceLock::new();

fn cache_writer() -> SyncSender<CacheWriterMsg> {
    CACHE_WRITER_TX
        .get_or_init(|| {
            let (tx, rx) = sync_channel::<CacheWriterMsg>(64);
            let _ = std::thread::Builder::new()
                .name(obfstr!("sniffer-cache-writer").to_string())
                .spawn(move || run_cache_writer(rx));
            tx
        })
        .clone()
}

/// Defers the cache write for `path` until the UI has been stable for
/// [`CACHE_WRITE_DEBOUNCE`]. The latest tree state is read at write time, so
/// intermediate states are never serialized at all.
fn schedule_cache_write(path: PathBuf, tree: SharedUiTree) {
    // A dropped message is harmless: the pending entry from an earlier call still
    // exists and snapshots the same (newer) tree when its own timer expires.
    let _ = cache_writer().try_send(CacheWriterMsg::Schedule { path, tree });
}

/// Persists a plugin's pending cache write immediately.
///
/// Called on plugin unload so shutdown cannot lose the final UI state to the
/// debounce timer.
pub fn flush_cache_write_now(path: &Path) {
    let _ = cache_writer().try_send(CacheWriterMsg::FlushNow {
        path: path.to_path_buf(),
    });
}

fn run_cache_writer(rx: Receiver<CacheWriterMsg>) {
    // One pending entry per cache path, always tracking the newest tree handle and
    // the latest deadline. Older snapshots are simply replaced.
    let mut pending: HashMap<PathBuf, (SharedUiTree, Instant)> = HashMap::new();
    loop {
        let next_due = pending.values().map(|(_, due)| *due).min();
        // A zero wait is valid: `recv_timeout` then times out immediately, which
        // drives the processing of entries that are due right now.
        let timeout = next_due.map_or(CACHE_WRITER_IDLE_WAIT, |due| {
            due.saturating_duration_since(Instant::now())
        });
        match rx.recv_timeout(timeout) {
            Ok(CacheWriterMsg::Schedule { path, tree }) => {
                pending.insert(path, (tree, Instant::now() + CACHE_WRITE_DEBOUNCE));
            }
            Ok(CacheWriterMsg::FlushNow { path }) => {
                if let Some((tree, _)) = pending.remove(&path) {
                    write_tree_snapshot(&path, &tree);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                let due: Vec<PathBuf> = pending
                    .iter()
                    .filter(|(_, (_, due))| *due <= now)
                    .map(|(path, _)| path.clone())
                    .collect();
                for path in due {
                    if let Some((tree, _)) = pending.remove(&path) {
                        write_tree_snapshot(&path, &tree);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                for (path, (tree, _)) in pending.drain() {
                    write_tree_snapshot(&path, &tree);
                }
                break;
            }
        }
    }
}

/// Serializes the current tree state and writes it to `path`.
///
/// The plugin lock is only held for a handle clone; the (potentially large)
/// `postcard` serialization runs on the cache writer thread without blocking the
/// UI thread.
fn write_tree_snapshot(path: &Path, tree: &SharedUiTree) {
    let snapshot = match tree.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };
    let Some(element) = snapshot else {
        return;
    };
    if let Ok(bytes) = postcard::to_allocvec(element.as_ref()) {
        let _ = std::fs::write(path, bytes);
    }
}

pub enum UiMutation {
    /// Replaces the whole tree. Shared instead of owned so the freshly parsed tree
    /// does not have to be deep-cloned on the way into the mutation buffer.
    SetUi(Arc<Element>),
    UpdateStyle {
        id: String,
        property: String,
        value: String,
    },
    SetText {
        id: String,
        text: String,
    },
    InsertChild {
        parent_id: String,
        /// Boxed to keep the mutation buffer small; the child is moved into the tree
        /// on flush, so the extra indirection costs one allocation and no copy.
        child: Box<Element>,
    },
    RemoveNode {
        id: String,
    },
}

#[derive(Default)]
pub struct MutationBuffer {
    pub mutations: Mutex<Vec<UiMutation>>,
}

impl MutationBuffer {
    pub fn flush(&self, ui_tree: &Arc<Mutex<Option<Arc<Element>>>>) {
        let mut muts = match self.mutations.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if muts.is_empty() {
            return;
        }

        if let Ok(mut tree_lock) = ui_tree.lock() {
            for m in muts.drain(..) {
                match m {
                    UiMutation::SetUi(new_elem) => {
                        // The shared tree is replaced as-is; no deep copy happens here.
                        *tree_lock = Some(new_elem);
                    }
                    UiMutation::UpdateStyle {
                        id,
                        property,
                        value,
                    } => {
                        if let Some(root) = tree_lock.as_mut() {
                            Arc::make_mut(root).mutate_style(&id, &property, &value);
                        }
                    }
                    UiMutation::SetText { id, text } => {
                        if let Some(root) = tree_lock.as_mut() {
                            Arc::make_mut(root).mutate_text(&id, &text);
                        }
                    }
                    UiMutation::InsertChild { parent_id, child } => {
                        if let Some(root) = tree_lock.as_mut() {
                            Arc::make_mut(root).insert_child(&parent_id, *child);
                        }
                    }
                    UiMutation::RemoveNode { id } => {
                        if let Some(root) = tree_lock.as_mut() {
                            Arc::make_mut(root).remove_node(&id);
                        }
                    }
                }
            }
            sniffer_core::types::UI_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            crate::js::host_bridge::wake();
        }
    }
}

#[allow(clippy::too_many_lines)]
pub fn register_ui_bindings<'js>(
    ctx: &Ctx<'js>,
    globals: &Object<'js>,
    ui_tree: Arc<Mutex<Option<Arc<Element>>>>,
    plugin_permissions: &[String],
    granted_permissions: Arc<Mutex<Vec<String>>>,
    cache_path: Option<std::path::PathBuf>,
) {
    let mutation_buffer = Arc::new(MutationBuffer::default());

    // Shared UI mutation committer
    let mut_set_ui = mutation_buffer.clone();
    let tree_set_ui = ui_tree.clone();
    let cache_path_apply = cache_path;
    let apply_element = Arc::new(move |parsed: Element| {
        // Build the shared tree once: the mutation buffer takes ownership of this
        // handle, so no deep copy of the element tree is made on the way in.
        let parsed = Arc::new(parsed);
        if let Ok(mut lock) = mut_set_ui.mutations.lock() {
            lock.push(UiMutation::SetUi(parsed));
        }
        mut_set_ui.flush(&tree_set_ui);
        if let Some(path) = cache_path_apply.clone() {
            // Debounced persistence: no serialization and no disk write here. The
            // cache writer snapshots the shared tree (which now contains this
            // update) once the UI has been stable for the debounce period.
            schedule_cache_write(path, Arc::clone(&tree_set_ui));
        }
    });

    // host_set_ui (legacy JSON string binder)
    let apply_ui = Arc::clone(&apply_element);
    let plugin_permissions_ui = plugin_permissions.to_vec();
    let granted_permissions_ui = granted_permissions.clone();
    let set_ui_func = Function::new(ctx.clone(), move |json_str: String| {
        if !permission_granted(
            obfstr!("plugin.permission.UI"),
            &plugin_permissions_ui,
            &granted_permissions_ui,
        ) {
            return;
        }

        match serde_json::from_str::<Element>(&json_str) {
            Ok(parsed) => {
                apply_ui(parsed);
            }
            Err(e) => {
                let col = e.column();
                let start = col.saturating_sub(50);
                let end = (col + 50).min(json_str.len());
                let snippet = if start < json_str.len() {
                    &json_str[start..end]
                } else {
                    ""
                };
                dev_err!(
                    "{}: {e} (around col {col}: '{snippet}')",
                    obfstr!("JS Error: Failed to parse host_set_ui JSON")
                );
            }
        }
    })
    .unwrap();
    globals.set(obfstr!("host_set_ui"), set_ui_func).unwrap();

    // host_set_ui_fast (Stage 6.3 direct serde deserializer bypassing JSON.stringify)
    let apply_ui_fast = Arc::clone(&apply_element);
    let plugin_permissions_ui_fast = plugin_permissions.to_vec();
    let granted_permissions_ui_fast = granted_permissions.clone();
    let set_ui_fast_func = Function::new(ctx.clone(), move |ctx: Ctx<'js>, val: Value<'js>| {
        if !permission_granted(
            obfstr!("plugin.permission.UI"),
            &plugin_permissions_ui_fast,
            &granted_permissions_ui_fast,
        ) {
            return;
        }

        // Fast path 1: If caller passed a JSON string (backward compatibility fallback)
        if let Some(s) = val.as_string() {
            match s.to_string() {
                Ok(json_str) => match serde_json::from_str::<Element>(&json_str) {
                    Ok(parsed) => {
                        apply_ui_fast(parsed);
                    }
                    Err(e) => {
                        dev_err!(
                            "{}: {e}",
                            obfstr!("JS Error: Failed to parse host_set_ui_fast JSON string")
                        );
                    }
                },
                Err(e) => {
                    dev_err!(
                        "{}: {e}",
                        obfstr!("JS Error: Failed to extract host_set_ui_fast string")
                    );
                }
            }
            return;
        }

        // Fast path 2: Direct deserialization from QuickJS Value AST using rquickjs_serde (Stage 6.3)
        // Completely bypasses JSON.stringify and intermediate heap string allocations
        match rquickjs_serde::from_value::<Element>(val.clone()) {
            Ok(parsed) => {
                apply_ui_fast(parsed);
            }
            Err(serde_err) => {
                // Fallback: If direct serde deserialization encountered an edge case, fallback to ctx.json_stringify
                if let Ok(Some(js_str)) = ctx.json_stringify(val)
                    && let Ok(json_str) = js_str.to_string()
                    && let Ok(parsed) = serde_json::from_str::<Element>(&json_str)
                {
                    apply_ui_fast(parsed);
                } else {
                    dev_err!(
                        "{}: {serde_err}",
                        obfstr!("JS Error: Failed to deserialize host_set_ui_fast value")
                    );
                }
            }
        }
    })
    .unwrap();
    globals
        .set(obfstr!("host_set_ui_fast"), set_ui_fast_func)
        .unwrap();

    // host_update_style
    let mut_update_style = mutation_buffer.clone();
    let tree_update_style = ui_tree.clone();
    let plugin_permissions_style = plugin_permissions.to_vec();
    let granted_permissions_style = granted_permissions.clone();
    let update_style_func = Function::new(
        ctx.clone(),
        move |id: String, property: String, value: String| {
            if !permission_granted(
                obfstr!("plugin.permission.UI"),
                &plugin_permissions_style,
                &granted_permissions_style,
            ) {
                return;
            }

            if let Ok(mut lock) = mut_update_style.mutations.lock() {
                lock.push(UiMutation::UpdateStyle {
                    id,
                    property,
                    value,
                });
            }
            mut_update_style.flush(&tree_update_style);
        },
    )
    .unwrap();
    globals
        .set(obfstr!("host_update_style"), update_style_func)
        .unwrap();

    // host_set_text
    let mut_set_text = mutation_buffer.clone();
    let tree_set_text = ui_tree.clone();
    let plugin_permissions_text = plugin_permissions.to_vec();
    let granted_permissions_text = granted_permissions.clone();
    let set_text_func = Function::new(ctx.clone(), move |id: String, text: String| {
        if !permission_granted(
            obfstr!("plugin.permission.UI"),
            &plugin_permissions_text,
            &granted_permissions_text,
        ) {
            return;
        }

        if let Ok(mut lock) = mut_set_text.mutations.lock() {
            lock.push(UiMutation::SetText { id, text });
        }
        mut_set_text.flush(&tree_set_text);
    })
    .unwrap();
    globals
        .set(obfstr!("host_set_text"), set_text_func)
        .unwrap();

    // host_insert_child
    let mut_insert_child = mutation_buffer.clone();
    let tree_insert_child = ui_tree.clone();
    let plugin_permissions_insert = plugin_permissions.to_vec();
    let granted_permissions_insert = granted_permissions.clone();
    let insert_child_func =
        Function::new(ctx.clone(), move |parent_id: String, child_json: String| {
            if !permission_granted(
                obfstr!("plugin.permission.UI"),
                &plugin_permissions_insert,
                &granted_permissions_insert,
            ) {
                return;
            }

            if let Ok(parsed_child) = serde_json::from_str::<Element>(&child_json) {
                if let Ok(mut lock) = mut_insert_child.mutations.lock() {
                    lock.push(UiMutation::InsertChild {
                        parent_id,
                        child: Box::new(parsed_child),
                    });
                }
                mut_insert_child.flush(&tree_insert_child);
            } else {
                dev_err!(
                    "{}",
                    obfstr!("JS Error: Failed to parse child JSON in host_insert_child")
                );
            }
        })
        .unwrap();
    globals
        .set(obfstr!("host_insert_child"), insert_child_func)
        .unwrap();

    // host_remove_node
    let mut_remove_node = mutation_buffer;
    let tree_remove_node = ui_tree;
    let plugin_permissions_remove = plugin_permissions.to_vec();
    let granted_permissions_remove = granted_permissions;
    let remove_node_func = Function::new(ctx.clone(), move |id: String| {
        if !permission_granted(
            obfstr!("plugin.permission.UI"),
            &plugin_permissions_remove,
            &granted_permissions_remove,
        ) {
            return;
        }

        if let Ok(mut lock) = mut_remove_node.mutations.lock() {
            lock.push(UiMutation::RemoveNode { id });
        }
        mut_remove_node.flush(&tree_remove_node);
    })
    .unwrap();
    globals
        .set(obfstr!("host_remove_node"), remove_node_func)
        .unwrap();

    // host_get_binary_state
    fn get_binary_state<'js>(ctx: Ctx<'js>) -> rquickjs::Result<rquickjs::ArrayBuffer<'js>> {
        let state = sniffer_core::types::AppState {
            click_count: 42,
            screen_width: f32::from_bits(
                sniffer_core::types::SCREEN_WIDTH.load(std::sync::atomic::Ordering::Relaxed),
            ),
            screen_height: f32::from_bits(
                sniffer_core::types::SCREEN_HEIGHT.load(std::sync::atomic::Ordering::Relaxed),
            ),
        };
        let bytes = postcard::to_allocvec(&state).unwrap_or_default();
        rquickjs::ArrayBuffer::new(ctx, bytes)
    }
    let get_binary_state_func = Function::new(ctx.clone(), get_binary_state).unwrap();
    globals
        .set(obfstr!("host_get_binary_state"), get_binary_state_func)
        .unwrap();

    // host_send_binary_event
    let send_binary_event_func = Function::new(ctx.clone(), |buffer: rquickjs::ArrayBuffer<'_>| {
        // SAFETY: Synchronous access to ArrayBuffer within single-threaded QuickJS execution.
        let bytes_opt = unsafe { buffer.as_bytes() };
        if let Some(bytes) = bytes_opt {
            if let Ok(state) = postcard::from_bytes::<sniffer_core::types::AppState>(bytes) {
                dev_log!(
                    "{}: {state:?}",
                    obfstr!("JS sent binary state via ArrayBuffer")
                );
            } else {
                dev_err!("{}", obfstr!("Failed to deserialize binary event from JS."));
            }
        }
    })
    .unwrap();
    globals
        .set(obfstr!("host_send_binary_event"), send_binary_event_func)
        .unwrap();
}

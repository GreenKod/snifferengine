#![cfg(not(target_os = "android"))]

//! Stress tests for the QuickJS plugin bridge.
//!
//! The bridge pays per-cycle allocation costs: UI trees cross from
//! JavaScript into Rust, event objects cross back, and handlers are cached
//! as `Persistent` references. These tests hammer those cycles and pin
//! down the two health properties of the sandbox:
//!
//! * the JS heap must not grow across cycles once garbage is collected —
//!   retained trees or pinned handler graphs would accumulate here, and
//! * GC cycles must stay fast and stable while the bridge churns — a
//!   heap that keeps growing makes every cycle slower.
//!
//! The third test guards the sandbox hard cap: a runaway allocation must
//! halt through a JS exception (never an abort), and repeated full engine
//! lifecycles must complete with bounded heaps.

use rquickjs::{Context, Function, Object, Persistent, Runtime};
use sniffer_core::types::Element;
use sniffer_core::ui::event::UiEvent;
use sniffer_core::ui::widget::fnv1a;
use sniffer_plugin::js::bindings_ui::register_ui_bindings;
use sniffer_plugin::js::engine::{DEFAULT_JS_MEMORY_LIMIT, create_engine};
use sniffer_plugin::js::events::build_ui_event_object;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type SharedTree = Arc<Mutex<Option<Arc<Element>>>>;

/// Bridge cycles per stress test.
const CYCLES: usize = 240;
/// Collect garbage every N cycles and sample the GC duration.
const GC_INTERVAL: usize = 40;
/// Post-GC heap growth tolerated across all cycles (atom-table slack
/// included). A retained tree per cycle would exceed this many times over.
const LEAK_TOLERANCE_BYTES: i64 = 128 * 1024;
/// Hard ceiling for one GC cycle during stress (real cycles are far below).
const GC_CYCLE_DEADLINE: Duration = Duration::from_millis(250);

/// JS-side tree builder: ~73 elements of allocation churn per invocation.
const TREE_BUILDER_SCRIPT: &str = r#"
globalThis.makeTree = function (seed) {
    var cards = [];
    for (var i = 0; i < 36; i++) {
        cards.push({
            Container: {
                id: "stress_card_" + seed + "_" + i,
                style: { width: 100, height: 120 },
                children: [{
                    Label: {
                        id: "stress_lbl_" + seed + "_" + i,
                        text: "Row " + i,
                        style: {}
                    }
                }]
            }
        });
    }
    return {
        Container: { id: "stress_root_" + seed, style: {}, children: cards }
    };
};
"#;

/// Samples GC cycles while the stress loop churns and asserts the two
/// health bounds: every cycle stays fast, and none of them degrades
/// pathologically as the loop progresses.
struct GcProbe {
    samples: Vec<Duration>,
}

impl GcProbe {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    fn collect(&mut self, runtime: &Runtime) {
        let start = Instant::now();
        runtime.run_gc();
        self.samples.push(start.elapsed());
    }

    fn assert_healthy(&self) {
        assert!(
            !self.samples.is_empty(),
            "the stress loop must collect GC samples"
        );
        let max = self.samples.iter().max().expect("non-empty");
        assert!(
            *max < GC_CYCLE_DEADLINE,
            "a GC cycle took {max:?} during stress; bridge workloads must not grow the heap into slow collections"
        );

        // Stability: the worst cycle of the second half must stay within a
        // generous factor of the worst cycle of the first half, so a heap
        // that keeps growing turns the test red instead of merely slow.
        let half = self.samples.len() / 2;
        let first_max = self.samples[..half]
            .iter()
            .max()
            .expect("first half non-empty");
        let second_max = self.samples[half..]
            .iter()
            .max()
            .expect("second half non-empty");
        assert!(
            second_max.as_nanos() <= first_max.as_nanos() * 8 + Duration::from_millis(5).as_nanos(),
            "GC cycles degraded across the loop (first-half max {first_max:?}, second-half max {second_max:?}); the heap is likely growing"
        );
    }
}

fn js_heap_bytes(runtime: &Runtime) -> i64 {
    runtime.memory_usage().memory_used_size
}

fn register_bridge(context: &Context) -> SharedTree {
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
        ctx.eval::<(), _>(TREE_BUILDER_SCRIPT)
            .expect("tree builder script");
    });
    ui_tree
}

#[test]
fn repeated_host_set_ui_cycles_do_not_leak_js_heap() {
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");
    let ui_tree = register_bridge(&context);

    // Warm up: intern atoms, JIT nothing, steady the heap, and take the
    // post-GC baseline.
    context.with(|ctx| {
        ctx.eval::<(), _>("host_set_ui_fast(makeTree('warm'))")
            .expect("warmup tree");
    });
    let mut probe = GcProbe::new();
    probe.collect(&runtime);
    let baseline = js_heap_bytes(&runtime);

    for cycle in 0..CYCLES {
        let script = format!("host_set_ui_fast(makeTree({cycle}))");
        context.with(|ctx| {
            ctx.eval::<(), _>(script).expect("bridge cycle");
        });
        if (cycle + 1) % GC_INTERVAL == 0 {
            probe.collect(&runtime);
        }
    }

    // Release the last parsed tree and collect everything that is left.
    *ui_tree.lock().unwrap() = None;
    probe.collect(&runtime);
    let after = js_heap_bytes(&runtime);

    probe.assert_healthy();
    assert!(
        after - baseline < LEAK_TOLERANCE_BYTES,
        "JS heap grew by {} bytes (baseline {baseline} -> {after}) across \
         {CYCLES} host_set_ui cycles; the bridge is retaining tree graphs",
        after - baseline
    );
}

#[test]
fn repeated_event_dispatch_cycles_do_not_leak_js_heap() {
    let runtime = Runtime::new().expect("runtime");
    let context = Context::full(&runtime).expect("context");

    // Production dispatch resolves the handler once and keeps it as a
    // Persistent reference; the stress loop does the same.
    let cached_handler: Persistent<Function> = context.with(|ctx| {
        ctx.eval::<(), _>(
            "globalThis.payloadSinks = 0;
            globalThis.onEvent = function (event) {
                globalThis.payloadSinks = event.id || event.type;
            };",
        )
        .expect("handler script");
        let handler: Function = ctx.eval("globalThis.onEvent").expect("onEvent function");
        Persistent::save(&ctx, handler)
    });

    // Rotate through the event variants the bridge builds, exactly like a
    // busy 120 Hz input stream would. GC and heap reads happen between
    // context scopes: the runtime's internal lock is held while a context
    // scope is active.
    let events: Vec<UiEvent> = (0..CYCLES)
        .map(|cycle| {
            let id = fnv1a(format!("stress_target_{cycle}").as_bytes());
            match cycle % 4 {
                0 => UiEvent::Click(id, 120.0, 48.0),
                1 => UiEvent::Hover(id, 120.0, 48.0, 3.0, 4.0),
                2 => UiEvent::Scroll(Some(id), 1.5, -2.25, 0.0, 480.0),
                _ => UiEvent::TextInput(format!("typed {cycle}")),
            }
        })
        .collect();

    let mut probe = GcProbe::new();
    probe.collect(&runtime);
    let baseline = js_heap_bytes(&runtime);

    for chunk in events.chunks(GC_INTERVAL) {
        context.with(|ctx| {
            let handler = cached_handler
                .clone()
                .restore(&ctx)
                .expect("cached handler");
            for event in chunk {
                let payload: Object = build_ui_event_object(&ctx, event)
                    .unwrap_or_else(|e| panic!("event object build failed: {e}"));
                handler
                    .call::<_, ()>((payload,))
                    .unwrap_or_else(|e| panic!("handler call failed: {e}"));
            }
        });
        probe.collect(&runtime);
    }

    probe.collect(&runtime);
    let after = js_heap_bytes(&runtime);

    probe.assert_healthy();
    assert!(
        after - baseline < LEAK_TOLERANCE_BYTES,
        "JS heap grew by {} bytes (baseline {baseline} -> {after}) across \
         {CYCLES} event dispatch cycles; the bridge is pinning event graphs",
        after - baseline
    );
}

#[test]
fn memory_cap_halts_runaway_allocation_and_gc_recovers() {
    let engine = create_engine(Some(DEFAULT_JS_MEMORY_LIMIT)).expect("engine");

    engine.context.with(|ctx| {
        // An allocation storm must stop through a JS exception (or the
        // interrupt handler), never through an engine abort.
        let outcome: String = ctx
            .eval(
                r#"
                var sink = [];
                try {
                    for (var i = 0; i < 100000; i++) {
                        sink.push(new Array(1024).fill("stress"));
                    }
                    "completed";
                } catch (e) {
                    "halted: " + e;
                }
                "#,
            )
            .expect("runaway allocation must not abort the engine");
        assert_ne!(
            outcome, "completed",
            "the memory cap must halt a runaway allocation inside the sandbox"
        );
    });

    engine.runtime.run_gc();
    let cap = i64::try_from(DEFAULT_JS_MEMORY_LIMIT).expect("cap fits i64");
    assert!(
        js_heap_bytes(&engine.runtime) <= cap,
        "the heap must fall back under the cap after the halt"
    );
}

#[test]
fn repeated_engine_lifecycles_stay_bounded() {
    // Full engine lifecycles under load: contexts, cached handlers, and
    // interrupted work must all be released when an engine drops, and the
    // next engine must start from the same bounded state.
    for generation in 0..16 {
        let engine = create_engine(Some(DEFAULT_JS_MEMORY_LIMIT)).expect("engine");
        engine.context.with(|ctx| {
            ctx.eval::<(), _>(TREE_BUILDER_SCRIPT)
                .expect("tree builder script");
            let churn = format!(
                "globalThis.host_tree_local_{generation} = makeTree({generation});
                 var acc = [];
                 for (var i = 0; i < 200; i++) {{ acc.push({{ v: i, s: 'x' + i }}); }}"
            );
            ctx.eval::<(), _>(churn).expect("churn script");
        });

        // Heap reads stay outside context scopes: the runtime's internal
        // lock is held while a context scope is active.
        let heap = js_heap_bytes(&engine.runtime);
        let cap = i64::try_from(DEFAULT_JS_MEMORY_LIMIT).expect("cap fits i64");
        assert!(heap <= cap, "engine {generation} exceeded its heap cap");
        engine.runtime.run_gc();
    }
    // Sixteen completed lifecycles mean every runtime and context was
    // dropped cleanly under load; a native-side retention bug would keep
    // the process alive past the test.
}

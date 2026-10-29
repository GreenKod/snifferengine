# SnifferEngine

General-purpose, declarative 120Hz embedded UI, physics, and graphics engine written in Rust with a QuickJS (`SnifferUI`) plugin runtime.

## Workspace Structure

- `crates/sniffer_engine`: Unified facade crate (`core`, `render`, `plugin`, `pkg`, `platform`, `prelude`).
- `crates/sniffer_core`: Declarative UI AST (`Element`), Flexbox layout (`Taffy`), 120Hz spring/fling/scroll physics, and `TransitionManager`.
- `crates/sniffer_render`: OpenGL ES / Glow renderer, SDF/Emoji font atlas, dual-Kawase backdrop blur, and geometry batching.
- `crates/sniffer_plugin`: Sandboxed QuickJS runtime, host bridge, and SHA-256 verified plugin loader.
- `crates/sniffer_pkg`: Native package & manifest verification utilities.
- `crates/sniffer_platform_android`: Android `NativeActivity` 120Hz event loop, touch physics, and JNI window insets bridge.
- `crates/sniffer_platform_desktop`: Desktop (`winit` + `glutin`) simulator and visual smoke runner.
- `packages/`: Built-in native engine packages (`scroll_view`, `perf_monitor`).
- `framework/`: `SnifferUI` declarative JavaScript UI framework (`sniffer_ui.js`, `core.js`, `types.d.ts`).


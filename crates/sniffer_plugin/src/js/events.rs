//! Event payload bridge between Rust [`UiEvent`]s and the QuickJS `onEvent` handler.
//!
//! Two payload formats are supported:
//!
//! * [`EventPayloadFormat::JsonString`] — the legacy contract. The host serializes the
//!   event into a JSON string and the handler parses it itself (`JSON.parse`). This is
//!   the default for any plugin that installs its own `onEvent`.
//! * [`EventPayloadFormat::Object`] — the fast bridge. The host builds the QuickJS
//!   object directly, so no intermediate string is allocated and no parse pass runs.
//!
//! The fast bridge is opt-in and is selected automatically when the plugin keeps the
//! framework dispatcher (`globalThis.__snifferDefaultOnEvent`) as `onEvent`, or when a
//! plugin explicitly declares `globalThis.__snifferEventFormat = "object"` after
//! installing an object-aware handler. Everything else keeps receiving JSON strings,
//! so existing plugins are unaffected.

use rquickjs::{Ctx, Object, Value};
use sniffer_core::ui::event::UiEvent;

/// How the host delivers a [`UiEvent`] to a plugin's `onEvent` handler.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventPayloadFormat {
    /// Legacy contract: a JSON string the handler has to parse itself.
    #[default]
    JsonString,
    /// Fast bridge: a ready-made QuickJS object without any string allocation.
    Object,
}

/// Global set by the framework (`core.js` / `sniffer_ui.js`) to the dispatcher it
/// installs as the default `onEvent`. Kept so the host can detect that a plugin did
/// not replace the framework handler.
pub const DEFAULT_HANDLER_GLOBAL: &str = "__snifferDefaultOnEvent";

/// Global a plugin may set to `"object"` to request the fast bridge for a custom
/// `onEvent` implementation that can consume structured events directly.
pub const PAYLOAD_FORMAT_GLOBAL: &str = "__snifferEventFormat";

/// Resolves the payload format for the currently installed `onEvent` handler.
///
/// Must run after the plugin scripts have been evaluated. Plugins that never touch
/// `onEvent` keep the framework dispatcher, which already accepts structured events,
/// so they get the fast bridge for free. Custom handlers fall back to the legacy JSON
/// string unless they opt in through [`PAYLOAD_FORMAT_GLOBAL`].
#[must_use]
pub fn detect_payload_format(ctx: &Ctx<'_>) -> EventPayloadFormat {
    let probe = format!(
        "(() => {{ \
            if (typeof globalThis.onEvent !== \"function\") return false; \
            if (globalThis.{PAYLOAD_FORMAT_GLOBAL} === \"object\") return true; \
            return typeof globalThis.{DEFAULT_HANDLER_GLOBAL} === \"function\" \
                && globalThis.onEvent === globalThis.{DEFAULT_HANDLER_GLOBAL}; \
        }})()"
    );
    match ctx.eval::<bool, _>(probe.into_bytes()) {
        Ok(true) => EventPayloadFormat::Object,
        _ => EventPayloadFormat::JsonString,
    }
}

/// Logs the dispatch of events that carry diagnostic value (currently clicks).
pub fn log_dispatch(event: &UiEvent, plugin_id: &str) {
    if let UiEvent::Click(id, w, h) = event {
        crate::logger::info(
            plugin_id,
            &format!("Dispatching Click: id={id}, w={w}, h={h}"),
        );
    }
}

/// Builds the QuickJS event object for the fast bridge.
///
/// Property names, value types and key order mirror [`serialize_ui_event_to_json`]
/// exactly, so a handler receives the same shape in both formats. `f32` geometry is
/// exposed as a JavaScript double, which is the same value the JSON parser produces.
pub fn build_ui_event_object<'js>(
    ctx: &Ctx<'js>,
    event: &UiEvent,
) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    match event {
        UiEvent::Click(id, w, h) => {
            obj.set("type", "Click")?;
            obj.set("id", id.to_string())?;
            obj.set("w", *w)?;
            obj.set("h", *h)?;
        }
        UiEvent::Hover(id, w, h, x, y) => {
            obj.set("type", "Hover")?;
            obj.set("id", id.to_string())?;
            obj.set("w", *w)?;
            obj.set("h", *h)?;
            obj.set("x", *x)?;
            obj.set("y", *y)?;
        }
        UiEvent::HoverEnd(id) => {
            obj.set("type", "HoverEnd")?;
            obj.set("id", id.to_string())?;
        }
        UiEvent::TextInput(text) => {
            obj.set("type", "TextInput")?;
            obj.set("text", text.as_str())?;
        }
        UiEvent::ClickOutside => {
            obj.set("type", "ClickOutside")?;
        }
        UiEvent::Backspace => {
            obj.set("type", "Backspace")?;
        }
        UiEvent::PointerDown(id) => {
            obj.set("type", "PointerDown")?;
            obj.set("id", id.to_string())?;
        }
        UiEvent::PointerUp(id) => {
            obj.set("type", "PointerUp")?;
            obj.set("id", optional_id(ctx, *id)?)?;
        }
        UiEvent::Scroll(id, dx, dy, max_x, max_y) => {
            obj.set("type", "Scroll")?;
            obj.set("id", optional_id(ctx, *id)?)?;
            obj.set("dx", *dx)?;
            obj.set("dy", *dy)?;
            obj.set("max_x", *max_x)?;
            obj.set("max_y", *max_y)?;
        }
        UiEvent::WindowResized(w, h) => {
            obj.set("type", "WindowResized")?;
            obj.set("w", *w)?;
            obj.set("h", *h)?;
        }
        UiEvent::PageSnapped { widget_id, page } => {
            obj.set("type", "PageSnapped")?;
            obj.set("widget_id", *widget_id)?;
            obj.set("page", *page)?;
        }
        UiEvent::SwipeUp => {
            obj.set("type", "SwipeUp")?;
        }
        UiEvent::SwipeDown => {
            obj.set("type", "SwipeDown")?;
        }
        UiEvent::Back => {
            obj.set("type", "Back")?;
        }
        UiEvent::ScrollDismiss {
            widget_id,
            displacement,
            velocity,
            action,
        } => {
            obj.set("type", "ScrollDismiss")?;
            obj.set("widget_id", *widget_id)?;
            obj.set("displacement", *displacement)?;
            obj.set("velocity", *velocity)?;
            obj.set("action", optional_string(ctx, action.as_deref())?)?;
        }
    }
    Ok(obj)
}

/// Serializes a [`UiEvent`] into a JSON string for the legacy `onEvent` contract.
#[must_use]
pub fn serialize_ui_event_to_json(event: &UiEvent) -> String {
    match event {
        UiEvent::Click(id, w, h) => {
            format!(r#"{{"type":"Click","id":"{id}","w":{w},"h":{h}}}"#)
        }
        UiEvent::Hover(id, w, h, x, y) => {
            format!(r#"{{"type":"Hover","id":"{id}","w":{w},"h":{h},"x":{x},"y":{y}}}"#)
        }
        UiEvent::HoverEnd(id) => format!(r#"{{"type":"HoverEnd","id":"{id}"}}"#),
        UiEvent::TextInput(text) => format!(
            r#"{{"type":"TextInput","text":{}}}"#,
            serde_json::to_string(text).unwrap_or_default()
        ),
        UiEvent::ClickOutside => r#"{"type":"ClickOutside"}"#.to_string(),
        UiEvent::Backspace => r#"{"type":"Backspace"}"#.to_string(),
        UiEvent::PointerDown(id) => format!(r#"{{"type":"PointerDown","id":"{id}"}}"#),
        UiEvent::PointerUp(id_opt) => {
            let id_str = optional_id_json(*id_opt);
            format!(r#"{{"type":"PointerUp","id":{id_str}}}"#)
        }
        UiEvent::Scroll(id_opt, dx, dy, max_x, max_y) => {
            let id_str = optional_id_json(*id_opt);
            format!(
                r#"{{"type":"Scroll","id":{id_str},"dx":{dx},"dy":{dy},"max_x":{max_x},"max_y":{max_y}}}"#
            )
        }
        UiEvent::WindowResized(w, h) => {
            format!(r#"{{"type":"WindowResized","w":{w},"h":{h}}}"#)
        }
        UiEvent::PageSnapped { widget_id, page } => {
            format!(r#"{{"type":"PageSnapped","widget_id":{widget_id},"page":{page}}}"#)
        }
        UiEvent::SwipeUp => r#"{"type":"SwipeUp"}"#.to_string(),
        UiEvent::SwipeDown => r#"{"type":"SwipeDown"}"#.to_string(),
        UiEvent::Back => r#"{"type":"Back"}"#.to_string(),
        UiEvent::ScrollDismiss {
            widget_id,
            displacement,
            velocity,
            action,
        } => {
            let action_str = match action {
                Some(a) => serde_json::to_string(a).unwrap_or_else(|_| "null".to_string()),
                None => "null".to_string(),
            };
            format!(
                r#"{{"type":"ScrollDismiss","widget_id":{widget_id},"displacement":{displacement},"velocity":{velocity},"action":{action_str}}}"#
            )
        }
    }
}

fn optional_id<'js>(ctx: &Ctx<'js>, id: Option<u64>) -> rquickjs::Result<Value<'js>> {
    match id {
        Some(id) => Ok(rquickjs::String::from_str(ctx.clone(), &id.to_string())?.into_value()),
        None => Ok(Value::new_null(ctx.clone())),
    }
}

fn optional_string<'js>(ctx: &Ctx<'js>, value: Option<&str>) -> rquickjs::Result<Value<'js>> {
    match value {
        Some(value) => Ok(rquickjs::String::from_str(ctx.clone(), value)?.into_value()),
        None => Ok(Value::new_null(ctx.clone())),
    }
}

fn optional_id_json(id_opt: Option<u64>) -> String {
    match id_opt {
        Some(id) => format!(r#""{id}""#),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};

    fn with_context(f: impl for<'js> FnOnce(&Ctx<'js>)) {
        let runtime = Runtime::new().expect("runtime");
        let context = Context::full(&runtime).expect("context");
        context.with(|ctx| f(&ctx));
    }

    /// Both bridge formats must expose the exact same JSON shape. Comparing the
    /// stringified object against the legacy payload guards against field renames,
    /// wrong value types (ids are strings, widget ids are numbers) and missing nulls.
    #[test]
    fn object_payload_matches_legacy_json_payload() {
        let events = [
            UiEvent::Click(42, 10.0, 20.5),
            UiEvent::Hover(7, 10.0, 20.0, 3.5, 4.5),
            UiEvent::HoverEnd(7),
            UiEvent::TextInput("hello \"world\"".to_string()),
            UiEvent::ClickOutside,
            UiEvent::Backspace,
            UiEvent::PointerDown(9),
            UiEvent::PointerUp(Some(9)),
            UiEvent::PointerUp(None),
            UiEvent::Scroll(Some(3), 1.5, -2.5, 100.0, 200.0),
            UiEvent::Scroll(None, 0.0, 0.0, 0.0, 0.0),
            UiEvent::WindowResized(1080.0, 2400.0),
            UiEvent::PageSnapped {
                widget_id: 11,
                page: 2,
            },
            UiEvent::SwipeUp,
            UiEvent::SwipeDown,
            UiEvent::Back,
            UiEvent::ScrollDismiss {
                widget_id: 12,
                displacement: 48.0,
                velocity: 1.25,
                action: Some("open".to_string()),
            },
            UiEvent::ScrollDismiss {
                widget_id: 12,
                displacement: 48.0,
                velocity: 1.25,
                action: None,
            },
        ];

        with_context(|ctx| {
            for event in &events {
                let legacy = serialize_ui_event_to_json(event);
                let object = build_ui_event_object(ctx, event).expect("object");
                ctx.globals()
                    .set("__payload_obj", object)
                    .expect("bind payload");
                let actual: String = ctx
                    .eval::<String, _>("JSON.stringify(__payload_obj)")
                    .unwrap_or_else(|_| "unstringifiable".to_string());
                let expected: String = ctx
                    .eval::<String, _>(format!("JSON.stringify({legacy})"))
                    .expect("legacy payload must be valid JSON");
                assert_eq!(actual, expected, "payload mismatch for {event:?}");
            }
        });
    }

    #[test]
    fn custom_handler_keeps_legacy_json_string_format() {
        with_context(|ctx| {
            ctx.eval::<(), _>("globalThis.onEvent = function (payload) { return payload; };")
                .expect("handler");
            assert_eq!(
                detect_payload_format(ctx),
                EventPayloadFormat::JsonString,
                "a plugin-provided onEvent must keep receiving JSON strings"
            );
        });
    }

    #[test]
    fn framework_dispatcher_opts_into_the_object_bridge() {
        with_context(|ctx| {
            ctx.eval::<(), _>(
                "globalThis.onEvent = function (event) { return event; }; \
                 globalThis.__snifferDefaultOnEvent = globalThis.onEvent;",
            )
            .expect("framework dispatcher");
            assert_eq!(detect_payload_format(ctx), EventPayloadFormat::Object);
        });
    }

    #[test]
    fn explicit_opt_in_enables_the_object_bridge() {
        with_context(|ctx| {
            ctx.eval::<(), _>(
                "globalThis.onEvent = function (event) { return event; }; \
                 globalThis.__snifferEventFormat = \"object\";",
            )
            .expect("opt-in");
            assert_eq!(detect_payload_format(ctx), EventPayloadFormat::Object);
        });
    }

    #[test]
    fn missing_handler_falls_back_to_json_strings() {
        with_context(|ctx| {
            assert_eq!(detect_payload_format(ctx), EventPayloadFormat::JsonString);
        });
    }
}

#![cfg(not(target_os = "android"))]

use sniffer_core::types::Action;
use sniffer_core::ui::data_map::DataMap;
use sniffer_core::ui::event::{EventBus, UiEvent};
use sniffer_core::ui::style_map::StyleMap;
use sniffer_core::ui::widget::{WidgetId, fnv1a};
use sniffer_plugin::{PluginRegistry, UiPlugin, WILDCARD_WIDGET_ID};
use std::sync::{Arc, Mutex};

/// Test double that records every event routed to it.
struct RecordingPlugin {
    subscriptions: Vec<WidgetId>,
    events: Arc<Mutex<Vec<UiEvent>>>,
}

impl RecordingPlugin {
    fn new(subscriptions: Vec<WidgetId>) -> Self {
        Self {
            subscriptions,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn received(&self) -> Vec<UiEvent> {
        self.events.lock().expect("event recorder poisoned").clone()
    }
}

impl UiPlugin for RecordingPlugin {
    fn subscriptions(&self) -> &[WidgetId] {
        &self.subscriptions
    }

    fn on_event(
        &self,
        event: &UiEvent,
        _styles: &StyleMap,
        _data: &DataMap,
        _actions: &Arc<Mutex<Vec<Action>>>,
    ) {
        self.events
            .lock()
            .expect("event recorder poisoned")
            .push(event.clone());
    }
}

fn dispatch(registry: &PluginRegistry, bus: &EventBus) {
    let actions = Arc::new(Mutex::new(Vec::new()));
    registry.dispatch(bus, &StyleMap::default(), &DataMap::default(), &actions);
}

fn register(registry: &mut PluginRegistry, plugin: &Arc<RecordingPlugin>) {
    registry.register(&(Arc::clone(plugin) as Arc<dyn UiPlugin>));
}

#[test]
fn widget_scoped_event_reaches_only_subscriber_and_wildcards() {
    let target = fnv1a(b"target_button");
    let mut registry = PluginRegistry::default();
    let direct = Arc::new(RecordingPlugin::new(vec![target]));
    let unrelated = Arc::new(RecordingPlugin::new(vec![fnv1a(b"other_button")]));
    let wildcard = Arc::new(RecordingPlugin::new(vec![WILDCARD_WIDGET_ID]));
    register(&mut registry, &direct);
    register(&mut registry, &unrelated);
    register(&mut registry, &wildcard);

    let bus = EventBus::default();
    bus.push(UiEvent::Click(target, 10.0, 20.0));
    dispatch(&registry, &bus);

    assert_eq!(direct.received().len(), 1);
    assert_eq!(wildcard.received().len(), 1);
    assert!(
        unrelated.received().is_empty(),
        "plugins subscribed to a different widget must not receive the event"
    );
}

#[test]
fn wildcard_subscriber_also_subscribed_to_widget_receives_event_once() {
    let target = fnv1a(b"shared_target");
    let mut registry = PluginRegistry::default();
    let plugin = Arc::new(RecordingPlugin::new(vec![target, WILDCARD_WIDGET_ID]));
    register(&mut registry, &plugin);

    let bus = EventBus::default();
    bus.push(UiEvent::PointerDown(target));
    dispatch(&registry, &bus);

    assert_eq!(
        plugin.received().len(),
        1,
        "direct + wildcard subscriptions must not duplicate delivery"
    );
}

#[test]
fn global_events_reach_every_registered_plugin() {
    let mut registry = PluginRegistry::default();
    let targeted = Arc::new(RecordingPlugin::new(vec![fnv1a(b"some_button")]));
    let untargeted = Arc::new(RecordingPlugin::new(Vec::new()));
    let wildcard = Arc::new(RecordingPlugin::new(vec![WILDCARD_WIDGET_ID]));
    register(&mut registry, &targeted);
    register(&mut registry, &untargeted);
    register(&mut registry, &wildcard);

    let bus = EventBus::default();
    bus.push(UiEvent::Back);
    bus.push(UiEvent::SwipeUp);
    bus.push(UiEvent::WindowResized(1080.0, 1920.0));
    dispatch(&registry, &bus);

    for plugin in [&targeted, &untargeted, &wildcard] {
        assert_eq!(
            plugin.received().len(),
            3,
            "global events must be delivered to every plugin"
        );
    }
}

#[test]
fn pointer_release_without_target_and_click_outside_are_global() {
    let mut registry = PluginRegistry::default();
    let direct = Arc::new(RecordingPlugin::new(vec![fnv1a(b"some_button")]));
    register(&mut registry, &direct);

    let bus = EventBus::default();
    bus.push(UiEvent::PointerUp(None));
    bus.push(UiEvent::ClickOutside);
    dispatch(&registry, &bus);

    assert_eq!(direct.received().len(), 2);
}

#[test]
fn page_and_dismiss_events_are_routed_by_widget_id() {
    let scroll = fnv1a(b"settings_scroll");
    let mut registry = PluginRegistry::default();
    let direct = Arc::new(RecordingPlugin::new(vec![scroll]));
    let unrelated = Arc::new(RecordingPlugin::new(vec![fnv1a(b"other_scroll")]));
    register(&mut registry, &direct);
    register(&mut registry, &unrelated);

    let bus = EventBus::default();
    bus.push(UiEvent::PageSnapped {
        widget_id: scroll,
        page: 2,
    });
    bus.push(UiEvent::ScrollDismiss {
        widget_id: scroll,
        displacement: 80.0,
        velocity: -600.0,
        action: Some("dismiss".to_string()),
    });
    dispatch(&registry, &bus);

    assert_eq!(direct.received().len(), 2);
    assert!(unrelated.received().is_empty());
}

#[test]
fn scroll_events_for_the_same_widget_are_still_coalesced() {
    let scroll = fnv1a(b"long_list");
    let mut registry = PluginRegistry::default();
    let direct = Arc::new(RecordingPlugin::new(vec![scroll]));
    register(&mut registry, &direct);

    let bus = EventBus::default();
    bus.push(UiEvent::Scroll(Some(scroll), 0.0, 10.0, 0.0, 500.0));
    bus.push(UiEvent::Scroll(Some(scroll), 4.0, 15.0, 0.0, 500.0));
    dispatch(&registry, &bus);

    let events = direct.received();
    assert_eq!(events.len(), 1, "same-widget scroll events must coalesce");
    match &events[0] {
        UiEvent::Scroll(Some(id), dx, dy, _, _) => {
            assert_eq!(*id, scroll);
            assert!((*dx - 4.0).abs() < f32::EPSILON);
            assert!((*dy - 25.0).abs() < f32::EPSILON);
        }
        other => panic!("expected coalesced Scroll event, got {other:?}"),
    }
}

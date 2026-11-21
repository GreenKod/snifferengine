use sniffer_core::anim::TransitionManager;
use sniffer_core::layout::LayoutNode;
use sniffer_core::math::{Rect, ScreenMetrics};
use sniffer_core::render::Renderer;
use sniffer_core::style::{ObjectFit, Style};
use sniffer_core::types::Element;
use sniffer_core::ui::{DataMap, StyleMap};
use sniffer_render::draw::draw_ui;
use sniffer_render::draw::image_schemes::{
    ImageProtocolHandler, register_image_scheme, request_scheme_image,
};
use std::sync::{Arc, Mutex};

/// `has_image` always answers false so the Image arm must fall back to the
/// registered custom-scheme handlers.
#[derive(Default)]
struct ImagelessRenderer;

impl Renderer for ImagelessRenderer {
    fn clear(&mut self, _color: u32) {}
    fn draw_rect(
        &mut self,
        _rect: Rect,
        _color: u32,
        _radius: f32,
        _border_width: f32,
        _border_color: Option<u32>,
    ) {
    }
    fn draw_rect_gradient(
        &mut self,
        _rect: Rect,
        _color_top: u32,
        _color_bottom: u32,
        _radius: f32,
        _border_width: f32,
        _border_color: Option<u32>,
    ) {
    }
    fn draw_shadow(
        &mut self,
        _rect: Rect,
        _radius: f32,
        _offset_y: f32,
        _spread: f32,
        _color: u32,
    ) {
    }
    fn draw_circle(&mut self, _cx: f32, _cy: f32, _radius: f32, _color: u32) {}
    fn draw_text(&mut self, _text: &str, _x: f32, _y: f32, _size: f32, _color: u32) {}
    fn begin_frame(&mut self, _width: f32, _height: f32) {}
    fn end_frame(&mut self) {}
    fn set_clip_rect(&mut self, _rect: Rect) {}
    fn clear_clip_rect(&mut self) {}
    fn pop_clip_rect(&mut self) {}
    fn current_clip(&self) -> Option<&sniffer_core::render::ClipRegion> {
        None
    }
    fn set_global_alpha(&mut self, _alpha: f32) {}
    fn load_image(&mut self, _id: &str, _rgba_pixels: &[u8], _width: u32, _height: u32) {}
    fn has_image(&self, _id: &str) -> bool {
        false
    }
    fn draw_image(&mut self, _id: &str, _rect: Rect, _radius: f32, _object_fit: ObjectFit) {}
    fn measure_text(&self, _text: &str, _size: f32) -> f32 {
        0.0
    }
}

struct RecordingSchemeHandler {
    requests: Mutex<Vec<String>>,
}

impl ImageProtocolHandler for RecordingSchemeHandler {
    fn scheme(&self) -> &'static str {
        "integration-test-icon"
    }

    fn request(&self, resource: &str) {
        self.requests.lock().unwrap().push(resource.to_string());
    }
}

fn draw_image_element(src: &str) -> usize {
    let mut renderer = ImagelessRenderer;
    let element = Element::Image {
        id: Some("scheme_image".to_string()),
        src: src.to_string(),
        style: Style::default(),
    };
    let layout = LayoutNode::new(0, None, Rect::new(10.0, 10.0, 48.0, 48.0));
    draw_ui(
        &mut renderer,
        &element,
        &layout,
        &ScreenMetrics::from_dpi(1080.0, 2400.0, 160.0),
        &StyleMap::default(),
        &DataMap::default(),
        &TransitionManager::default(),
        1.0,
        0.0,
        0.0,
    )
}

#[test]
fn image_element_delegates_custom_schemes_to_registered_handler() {
    let concrete = Arc::new(RecordingSchemeHandler {
        requests: Mutex::new(Vec::new()),
    });
    register_image_scheme(concrete.clone());

    let rendered = draw_image_element("integration-test-icon://com.example.app");
    assert_eq!(rendered, 1, "the unresolved image node itself is counted");
    let requests = concrete.requests.lock().unwrap();
    assert_eq!(
        *requests,
        vec!["com.example.app".to_string()],
        "the render layer must delegate the resource to the owning handler"
    );
}

#[test]
fn image_element_with_unregistered_scheme_degrades_gracefully() {
    let rendered = draw_image_element("nobody-registered-this://resource");
    assert_eq!(rendered, 1);
}

#[test]
fn direct_registry_lookup_rejects_plain_paths() {
    assert!(!request_scheme_image("assets/plain_path.png"));
}

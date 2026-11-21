use std::sync::{Arc, RwLock};

/// Resolves resources for one custom `scheme://` image source (app icons,
/// remote content, anything host-specific).
///
/// Handlers are registered by the embedding host with
/// [`register_image_scheme`], keeping host-specific scheme names out of the
/// engine: the render layer only ever asks "does any registered handler own
/// this URI?" and never names a scheme itself.
pub trait ImageProtocolHandler: Send + Sync {
    /// Scheme this handler owns, without the `://` suffix (e.g. `my-icons`).
    fn scheme(&self) -> &'static str;

    /// Asynchronously requests the resource; pixels are delivered later
    /// through `Renderer::load_image` under the full `scheme://resource` id,
    /// so a follow-up frame finds the image and draws it.
    fn request(&self, resource: &str);
}

static HANDLERS: RwLock<Vec<Arc<dyn ImageProtocolHandler>>> = RwLock::new(Vec::new());

/// Registers a handler for its scheme, replacing any previously registered
/// handler with the same scheme name.
pub fn register_image_scheme(handler: Arc<dyn ImageProtocolHandler>) {
    if let Ok(mut handlers) = HANDLERS.write() {
        handlers.retain(|existing| existing.scheme() != handler.scheme());
        handlers.push(handler);
    }
}

/// Requests `src` from the handler owning its scheme.
///
/// Returns `true` when a handler matched and the request is in flight;
/// `false` means no registered handler owns the scheme (the image simply
/// stays unresolved, exactly like an unknown plain path).
pub fn request_scheme_image(src: &str) -> bool {
    let Some((scheme, resource)) = split_scheme(src) else {
        return false;
    };
    if let Ok(handlers) = HANDLERS.read() {
        for handler in handlers.iter() {
            if handler.scheme() == scheme {
                handler.request(resource);
                return true;
            }
        }
    }
    false
}

/// Splits `scheme://resource` into its parts, rejecting empty components.
fn split_scheme(src: &str) -> Option<(&str, &str)> {
    let (scheme, resource) = src.split_once("://")?;
    if scheme.is_empty() || resource.is_empty() {
        return None;
    }
    Some((scheme, resource))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingHandler {
        scheme: &'static str,
        requests: Mutex<Vec<String>>,
    }

    impl ImageProtocolHandler for RecordingHandler {
        fn scheme(&self) -> &'static str {
            self.scheme
        }

        fn request(&self, resource: &str) {
            self.requests.lock().unwrap().push(resource.to_string());
        }
    }

    /// Registers a recording handler and keeps the concrete handle for
    /// assertions.
    fn register_recording(scheme: &'static str) -> Arc<RecordingHandler> {
        let handler = Arc::new(RecordingHandler {
            scheme,
            requests: Mutex::new(Vec::new()),
        });
        register_image_scheme(handler.clone());
        handler
    }

    #[test]
    fn dispatches_to_the_handler_owning_the_scheme() {
        let handler = register_recording("unit-test-icon");

        assert!(request_scheme_image("unit-test-icon://com.example.app"));
        let requests = handler.requests.lock().unwrap();
        assert_eq!(*requests, vec!["com.example.app".to_string()]);
    }

    #[test]
    fn returns_false_for_unregistered_and_malformed_sources() {
        assert!(!request_scheme_image("not-registered://resource"));
        assert!(!request_scheme_image("no-scheme-separator"));
        assert!(!request_scheme_image("://empty-scheme"));
        assert!(!request_scheme_image("unit-test-icon://"));
    }

    #[test]
    fn re_registration_replaces_the_previous_handler() {
        let first = register_recording("unit-test-replace");
        let second = register_recording("unit-test-replace");

        assert!(request_scheme_image("unit-test-replace://res"));
        assert!(first.requests.lock().unwrap().is_empty());
        let requests = second.requests.lock().unwrap();
        assert_eq!(*requests, vec!["res".to_string()]);
    }
}

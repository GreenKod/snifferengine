//! Virtual Page Window Manager & Element Recycling Pool
//!
//! Generic `ScrollView` content virtualization built on the declarative
//! pager contract: any `ScrollView` that declares pager mode (`snap_x` +
//! `page_count`) exposes its page row for window pruning, recycling, and
//! restoration. No element ID or naming convention is consulted anywhere.
//!
//! Off-screen page children move to `page_cache` and `ElementPool`;
//! the runtime window policy is centrally gated in
//! [`VirtualPageManager::virtualize_tree_with_velocity`].

use crate::types::Element;
use std::collections::{HashMap, HashSet, VecDeque};

/// Default page window radius entering Taffy layout (current ± 1)
pub const PAGE_WINDOW_RADIUS: usize = 1;

/// Maximum ElementPool capacity (56 elements = 2 pages of cards)
pub const POOL_MAX_CAPACITY: usize = 56;

/// Recycled Element Pool.
/// Off-screen card elements are preserved in this pool rather than being deallocated.
#[derive(Debug)]
pub struct ElementPool {
    pool: VecDeque<Element>,
    max_capacity: usize,
}

impl ElementPool {
    #[must_use]
    pub fn new(max_capacity: usize) -> Self {
        Self {
            pool: VecDeque::with_capacity(max_capacity),
            max_capacity,
        }
    }

    /// Recycles an element into the pool. Dropped via RAII when capacity is exceeded.
    pub fn recycle(&mut self, element: Element) {
        if self.pool.len() < self.max_capacity {
            self.pool.push_back(element);
        }
    }

    /// Recycles an entire batch of child elements into the pool.
    pub fn recycle_all(&mut self, children: Vec<Element>) {
        for child in children {
            self.recycle(child);
        }
    }

    /// Retrieves an available element from the pool.
    pub fn take(&mut self) -> Option<Element> {
        self.pool.pop_front()
    }

    /// Number of elements currently available in the pool.
    #[must_use]
    pub fn available(&self) -> usize {
        self.pool.len()
    }
}

impl Default for ElementPool {
    fn default() -> Self {
        Self::new(POOL_MAX_CAPACITY)
    }
}

/// Virtual page window manager.
///
/// Generic `ScrollView` pager virtualization: discovers the first
/// declarative pager (`snap_x` + `page_count`) in the tree and manages its
/// page window through `page_cache` and the recycled `element_pool`.
#[derive(Debug)]
pub struct VirtualPageManager {
    /// Stripped page children storage (page_index -> children)
    page_cache: HashMap<usize, Vec<Element>>,
    /// Recycled element pool
    pub element_pool: ElementPool,
    /// Currently materialized page indices
    #[allow(dead_code)]
    materialized: HashSet<usize>,
    /// Currently active page index
    current_page: usize,
    /// Total page count
    total_pages: usize,
    /// Whether virtualization is active
    active: bool,
}

impl Default for VirtualPageManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualPageManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            page_cache: HashMap::new(),
            element_pool: ElementPool::new(POOL_MAX_CAPACITY),
            materialized: HashSet::new(),
            current_page: 0,
            total_pages: 1,
            active: false,
        }
    }

    /// Returns the active page index.
    #[must_use]
    pub const fn current_page(&self) -> usize {
        self.current_page
    }

    /// Directly sets the active page index and synchronizes the tree.
    pub fn set_active_page(&mut self, page_idx: usize, root: &mut Element) {
        self.current_page = page_idx;
        self.virtualize_tree(root);
    }

    /// Standard virtualize_tree (assuming zero velocity).
    pub fn virtualize_tree(&mut self, root: &mut Element) {
        self.virtualize_tree_with_velocity(root, 0.0);
    }

    /// Virtualization gate with dynamic prefetch window based on scroll velocity.
    ///
    /// The page-window policy is intentionally dormant for now: with the
    /// declarative pager contract in place, mid-swipe recalculation spikes
    /// outweigh the gains for typical page counts, so the tree stays fully
    /// materialized. The planned visible-range culling (axis-aligned pruning
    /// of `ScrollView` content in the layout and render phases) supersedes
    /// page-window stripping and will hook in through this entry point.
    pub fn virtualize_tree_with_velocity(&mut self, _root: &mut Element, _velocity: f32) {
        self.active = false;
    }

    /// Triggers pre-materialization when scroll offset crosses 50% threshold.
    /// Returns true if the page window changed, triggering re-layout.
    pub fn update_predicted_page(
        &mut self,
        predicted_page: usize,
        root: &mut Element,
        velocity: f32,
    ) -> bool {
        self.current_page = predicted_page;
        self.virtualize_tree_with_velocity(root, velocity);
        false // Force false to prevent mid-swipe Taffy layout recalculation CPU spikes
    }

    /// Invoked when page snap completes or active page changes.
    pub fn on_page_changed(&mut self, new_page: usize, root: &mut Element) {
        self.current_page = new_page;
        self.virtualize_tree(root);
    }

    /// Invalidates caches on viewport resize or rotation,
    /// updating stub container dimensions and forcing re-virtualization.
    pub fn on_viewport_resized(&mut self, _new_w: f32, _new_h: f32, root: &mut Element) {
        self.restore_all_cached_pages(root);
        self.page_cache.clear();
        self.virtualize_tree(root);
    }

    /// Invalidates caches on viewport dimension or orientation changes.
    pub fn invalidate_on_resize(&mut self, root: &mut Element) {
        self.restore_all_cached_pages(root);
        self.page_cache.clear();
        self.virtualize_tree(root);
    }

    /// Restores every cached page into its slot in the paged track.
    fn restore_all_cached_pages(&mut self, root: &mut Element) {
        let cached_keys: Vec<usize> = self.page_cache.keys().copied().collect();
        if cached_keys.is_empty() {
            return;
        }
        if let Some(track_children) =
            Self::find_paged_track_children_mut(root, &mut self.total_pages)
        {
            for page_idx in cached_keys {
                if let Some(page_el) = track_children.get_mut(page_idx) {
                    self.restore_page(page_idx, page_el);
                }
            }
        }
    }

    /// Strips children of a specific page and moves them to cache or pool.
    /// The page container remains as an empty stub.
    pub fn strip_page(&mut self, page_idx: usize, page_el: &mut Element) {
        let children_to_strip = match page_el {
            Element::Container { children, .. } | Element::ScrollView { children, .. } => {
                if children.is_empty() {
                    return;
                }
                std::mem::take(children)
            }
            _ => return,
        };

        if self.page_cache.len() < 5 {
            self.page_cache.insert(page_idx, children_to_strip);
        } else {
            self.element_pool.recycle_all(children_to_strip);
        }
    }

    /// Restores children of a specific page from cache or pool.
    pub fn restore_page(&mut self, page_idx: usize, page_el: &mut Element) {
        let children_target = match page_el {
            Element::Container { children, .. } | Element::ScrollView { children, .. } => children,
            _ => return,
        };

        if !children_target.is_empty() {
            return;
        }

        if let Some(cached_children) = self.page_cache.remove(&page_idx) {
            *children_target = cached_children;
        }
    }

    /// Resolves the page-slot list of the first declarative pager in the
    /// tree.
    ///
    /// The pager contract is purely declarative: a `ScrollView` in pager
    /// mode carries `snap_x` (the page width) plus `page_count`. Its first
    /// child container is the page row, whose children are the page slots —
    /// the shape snap-scrolling already requires for horizontal layout. No
    /// element ID or naming convention is consulted.
    fn find_paged_track_children_mut<'a>(
        root: &'a mut Element,
        out_page_count: &mut usize,
    ) -> Option<&'a mut Vec<Element>> {
        let pager = Self::find_element_by_predicate_mut(root, &Self::is_declarative_pager)?;
        if let Element::ScrollView {
            children,
            page_count,
            ..
        } = pager
        {
            if let Some(pc) = page_count {
                *out_page_count = *pc as usize;
            }
            if let Some(Element::Container {
                children: track_children,
                ..
            }) = children.first_mut()
            {
                return Some(track_children);
            }
        }
        None
    }

    /// Matches `ScrollView`s that declare pager mode through the declarative
    /// `snap_x` + `page_count` fields.
    fn is_declarative_pager(element: &Element) -> bool {
        matches!(
            element,
            Element::ScrollView {
                snap_x: Some(_),
                page_count: Some(_),
                ..
            }
        )
    }

    /// Recursively finds the first element matching `predicate` (DFS).
    pub fn find_element_by_predicate_mut<'a>(
        element: &'a mut Element,
        predicate: &dyn Fn(&Element) -> bool,
    ) -> Option<&'a mut Element> {
        if predicate(element) {
            return Some(element);
        }

        match element {
            Element::Container { children, .. }
            | Element::ScrollView { children, .. }
            | Element::SharedView { children, .. } => {
                for child in children {
                    if let Some(found) = Self::find_element_by_predicate_mut(child, predicate) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// Recursively finds an element by ID within the AST tree (DFS).
    pub fn find_element_by_id_mut<'a>(
        element: &'a mut Element,
        target_id: &str,
    ) -> Option<&'a mut Element> {
        Self::find_element_by_predicate_mut(element, &|e| e.id() == Some(target_id))
    }
}

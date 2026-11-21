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

use crate::layout::LayoutNode;
use crate::style::{Dimension, PointerEvents, Style};
use crate::types::Element;
use crate::ui::widget::fnv1a;
use std::collections::{HashMap, HashSet, VecDeque};

/// Default page window radius entering Taffy layout (current ± 1)
pub const PAGE_WINDOW_RADIUS: usize = 1;

/// Maximum ElementPool capacity (56 elements = 2 pages of cards)
pub const POOL_MAX_CAPACITY: usize = 56;

/// Minimum flow-child count before a `ScrollView`'s content is window-culled.
/// Shorter lists stay fully materialized; culling them cannot pay for the
/// bookkeeping.
pub const CULLING_MIN_CHILDREN: usize = 24;

/// Materialized content margin beyond the visible range, in viewport
/// multiples per side.
///
/// Generous enough to survive a fast fling frame burst, tight enough to
/// keep relayouts proportional to the visible window.
pub const CULLING_MARGIN_VIEWPORTS: f32 = 0.75;

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

/// Bookkeeping for one window-culled `ScrollView` flow list.
#[derive(Debug)]
struct ScrollViewWindow {
    /// True when the flow axis is horizontal (a `snap_x` pager).
    horizontal: bool,
    /// Viewport extent along the flow axis, from the last measured layout.
    viewport: f32,
    /// Measured (main, cross) size of every flow child, index-aligned.
    child_sizes: Vec<(f32, f32)>,
    /// Main-axis start of every flow child in content coordinates.
    child_starts: Vec<f32>,
    /// Current materialized index window [start, end); `end` is exclusive.
    window_start: usize,
    window_end: usize,
}

impl ScrollViewWindow {
    /// Desired index range covering `scroll_pos` plus the configured margins.
    /// Returns the full range when the list is too short to cull.
    fn desired_range(&self, scroll_pos: f32) -> (usize, usize) {
        let count = self.child_starts.len();
        if count < CULLING_MIN_CHILDREN {
            return (0, count);
        }
        let margin = self.viewport * CULLING_MARGIN_VIEWPORTS;
        let span_start = scroll_pos - margin;
        let span_end = scroll_pos + self.viewport + margin;

        let mut lo = 0usize;
        while lo < count && self.child_starts[lo] + self.child_sizes[lo].0 < span_start {
            lo += 1;
        }
        let mut hi = count;
        while hi > lo && self.child_starts[hi - 1] > span_end {
            hi -= 1;
        }
        (lo, hi.max(lo))
    }
}

/// Virtual page window manager.
///
/// Generic `ScrollView` content virtualization: pagers declare themselves
/// through `snap_x` + `page_count`, and long flow lists are window-culled
/// around the live scroll position with size-preserving stubs, so both the
/// layout and render phases stay proportional to the visible window.
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
    /// Window bookkeeping per culled `ScrollView` (widget id keyed).
    scroll_windows: HashMap<u64, ScrollViewWindow>,
    /// Original children behind active stubs, keyed by (sv id, index).
    stripped_children: HashMap<(u64, usize), Element>,
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
            scroll_windows: HashMap::new(),
            stripped_children: HashMap::new(),
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
        self.restore_all_scroll_stubs(root);
        self.page_cache.clear();
        self.scroll_windows.clear();
        self.virtualize_tree(root);
    }

    /// Invalidates caches on viewport dimension or orientation changes.
    pub fn invalidate_on_resize(&mut self, root: &mut Element) {
        self.restore_all_cached_pages(root);
        self.restore_all_scroll_stubs(root);
        self.page_cache.clear();
        self.scroll_windows.clear();
        self.virtualize_tree(root);
    }

    /// Drops all scroll-culling bookkeeping without touching the tree.
    ///
    /// Called after a UI rebuild: the plugin emits a fresh, fully
    /// materialized tree, so any window state and cached originals are stale.
    pub fn reset_scroll_culling(&mut self) {
        self.scroll_windows.clear();
        self.stripped_children.clear();
    }

    /// Windows every cullable `ScrollView` around its current scroll
    /// position from the `offsets` map (widget id -> (pos_x, pos_y)).
    ///
    /// Returns `true` when any window changed. Stubs preserve every
    /// measured size, so geometry and scroll extents stay exact either way.
    pub fn refresh_scroll_windows(
        &mut self,
        root: &mut Element,
        layout: &LayoutNode,
        offsets: &HashMap<u64, (f32, f32)>,
    ) -> bool {
        let sv_ids: Vec<u64> = offsets.keys().copied().collect();
        let mut changed = false;
        for sv_id in sv_ids {
            let (pos_x, pos_y) = offsets[&sv_id];
            changed |= self.update_scroll_window(sv_id, (pos_x, pos_y), root, layout);
        }
        changed
    }

    /// Culls or restores the flow children of one `ScrollView` around
    /// `scroll_pos`, replacing off-window children with size-preserving
    /// stubs. Returns `true` when the materialized window changed.
    ///
    /// The layout and render phases stay proportional to the window:
    /// stubs are leaf containers with fixed geometry, no visuals, and no
    /// pointer events, and their measured footprints keep the scrollable
    /// content extent (and therefore physics clamping) exact.
    pub fn update_scroll_window(
        &mut self,
        sv_id: u64,
        scroll_pos: (f32, f32),
        root: &mut Element,
        layout: &LayoutNode,
    ) -> bool {
        let Some((flow_parent_element, horizontal, wrapper_holds_flow)) =
            Self::find_flow_list_mut(root, sv_id)
        else {
            return false;
        };
        let Some(flow_parent_layout) = Self::find_flow_layout(layout, sv_id, wrapper_holds_flow)
        else {
            return false;
        };

        let Some(children) = Self::children_of_mut(flow_parent_element) else {
            return false;
        };
        let Some(sv_layout) = layout.find_node_by_widget_id(sv_id) else {
            return false;
        };
        let layout_children = &flow_parent_layout.children;
        if children.len() != layout_children.len() || children.is_empty() {
            // Element and layout trees disagree (stale layout): measurement
            // is unsafe, keep the current materialization untouched.
            return false;
        }
        let count = children.len();

        // (Re)measure from the live layout: stubs keep their fixed sizes,
        // so measurements stay stable across cull cycles.
        let mut child_sizes = Vec::with_capacity(count);
        let mut child_starts = Vec::with_capacity(count);
        let mut min_start = f32::MAX;
        for child_lay in layout_children {
            let (start, main, cross) = if horizontal {
                (
                    child_lay.rect.x,
                    child_lay.rect.width,
                    child_lay.rect.height,
                )
            } else {
                (
                    child_lay.rect.y,
                    child_lay.rect.height,
                    child_lay.rect.width,
                )
            };
            child_starts.push(start);
            child_sizes.push((main, cross));
            if start < min_start {
                min_start = start;
            }
        }
        if min_start.is_finite() {
            for start in &mut child_starts {
                *start -= min_start;
            }
        }

        // The visible extent always comes from the scroll view itself, even
        // when a wrapper container holds the flow items (a pager's row spans
        // all pages, so its own width would defeat windowing).
        let viewport = if horizontal {
            sv_layout.rect.width
        } else {
            sv_layout.rect.height
        };
        let scroll_main = if horizontal {
            scroll_pos.0
        } else {
            scroll_pos.1
        };

        let window = self
            .scroll_windows
            .entry(sv_id)
            .or_insert(ScrollViewWindow {
                horizontal,
                viewport,
                child_sizes,
                child_starts,
                window_start: 0,
                window_end: count,
            });
        window.horizontal = horizontal;
        window.viewport = viewport;

        let (desired_start, desired_end) = window.desired_range(scroll_main);
        if desired_start == window.window_start && desired_end == window.window_end {
            return false;
        }

        let old_start = window.window_start;
        let old_end = window.window_end;
        window.window_start = desired_start;
        window.window_end = desired_end;

        // Strip newly off-window children into stubs, restore newly
        // on-window originals. Index arithmetic is identical for both
        // directions because the list length never changes.
        for idx in 0..count {
            let in_old = idx >= old_start && idx < old_end;
            let in_new = idx >= desired_start && idx < desired_end;
            if in_old == in_new {
                continue;
            }
            let Some(child) = children.get_mut(idx) else {
                continue;
            };
            if in_new {
                if let Some(original) = self.stripped_children.remove(&(sv_id, idx)) {
                    *child = original;
                }
            } else {
                let size = window.child_sizes[idx];
                let stub = Self::sized_stub(child, size, horizontal);
                self.stripped_children
                    .insert((sv_id, idx), std::mem::replace(child, stub));
            }
        }

        self.active = count >= CULLING_MIN_CHILDREN;
        true
    }

    /// Restores every stubbed scroll child, leaving the tree fully
    /// materialized. Used by the resize invalidation paths before
    /// re-measuring the whole tree.
    fn restore_all_scroll_stubs(&mut self, root: &mut Element) {
        if self.stripped_children.is_empty() {
            return;
        }
        let sv_ids: Vec<u64> = self.scroll_windows.keys().copied().collect();
        for sv_id in sv_ids {
            let Some(children) = Self::find_flow_list_mut(root, sv_id)
                .and_then(|(parent, _, _)| Self::children_of_mut(parent))
            else {
                // The scroll view left the tree; drop its cached originals.
                continue;
            };
            for (idx, child) in children.iter_mut().enumerate() {
                if let Some(original) = self.stripped_children.remove(&(sv_id, idx)) {
                    *child = original;
                }
            }
        }
        self.stripped_children.clear();
    }

    /// Resolves the mutable flow list of a scroll view in one traversal:
    /// its own children, or the children of its single wrapper container
    /// (the shape column and row layouts require).
    ///
    /// Returns the flow parent element, whether the flow axis is horizontal
    /// (`snap_x` pagers), and whether a wrapper container holds the flow.
    fn find_flow_list_mut(root: &mut Element, sv_id: u64) -> Option<(&mut Element, bool, bool)> {
        let sv = Self::find_element_by_predicate_mut(root, &|e: &Element| {
            e.id().is_some_and(|id| fnv1a(id.as_bytes()) == sv_id)
                && matches!(e, Element::ScrollView { .. })
        })?;

        // Shared fact-finding borrow ends before the mutable child access.
        let (horizontal, wrapper_holds_flow) = match sv {
            Element::ScrollView {
                snap_x, children, ..
            } => (
                snap_x.is_some(),
                children.len() == 1 && matches!(children.first(), Some(Element::Container { .. })),
            ),
            _ => return None,
        };

        if wrapper_holds_flow {
            let Element::ScrollView { children, .. } = sv else {
                return None;
            };
            children
                .first_mut()
                .map(|wrapper| (wrapper, horizontal, true))
        } else {
            Some((sv, horizontal, false))
        }
    }

    /// Resolves the layout node of the flow list matching the element-side
    /// resolution: the scroll view itself, or its first child.
    fn find_flow_layout(
        layout: &LayoutNode,
        sv_id: u64,
        wrapper_holds_flow: bool,
    ) -> Option<&LayoutNode> {
        let sv_layout = layout.find_node_by_widget_id(sv_id)?;
        if wrapper_holds_flow {
            sv_layout.children.first()
        } else {
            Some(sv_layout)
        }
    }

    /// Borrows the mutable child list of a container-shaped element.
    fn children_of_mut(element: &mut Element) -> Option<&mut Vec<Element>> {
        match element {
            Element::Container { children, .. }
            | Element::ScrollView { children, .. }
            | Element::SharedView { children, .. } => Some(children),
            _ => None,
        }
    }

    /// Builds a leaf stub that preserves the measured flow footprint of
    /// `original`: fixed geometry, copied margins and flex factors, no
    /// visuals, and no pointer events, so the stub is invisible to the
    /// renderer, the hit-test, and the scroll extents alike.
    ///
    /// `size` is (main, cross) along the flow axis; it maps back onto
    /// width/height according to `horizontal`.
    fn sized_stub(original: &Element, size: (f32, f32), horizontal: bool) -> Element {
        let (width, height) = if horizontal {
            (size.0, size.1)
        } else {
            (size.1, size.0)
        };
        let base = original.style();
        let style = Style {
            width: Dimension::Pixels(width),
            height: Dimension::Pixels(height),
            margin: base.margin,
            flex_grow: base.flex_grow,
            flex_shrink: base.flex_shrink,
            pointer_events: PointerEvents::None,
            ..Style::default()
        };
        Element::Container {
            id: original.id().map(str::to_string),
            style,
            children: Vec::new(),
        }
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

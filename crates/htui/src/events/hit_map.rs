//! Hit regions and spatial lookup over the laid-out box forest.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::{
    layout::{Rect, ScrollbarLayout, layoutbox::LayoutBox},
    tdom::node::{NodeArena, NodeId},
};

use super::{ElementRef, Geometry};

/// A rectangular hit region for one laid-out element.
///
/// `rect` is in canvas coordinates (column/row space); `path` is the DOM
/// ancestor chain of the element (itself first, then parents), used for
///
/// Every layout box becomes a region so CSS `:hover` rules can target any
/// element; elements without listeners simply fire nothing when dispatched.
#[derive(Debug, Clone)]
pub struct HitRegion {
    /// Session-local node id, used by hover enter / leave tracking to tell
    /// regions apart within one layout session. Not stable across `Redraw`s.
    pub node_id: NodeId,
    pub rect: Rect,
    /// The `clientWidth` / `clientHeight` box: this box's rect inset by its
    /// own border widths (content + padding, the browser client-box rule).
    pub client: Rect,
    /// `[self, parent, grandparent, …]` — the bubbling path.
    pub path: Arc<[ElementRef]>,
}

impl HitRegion {
    /// The element's stable `id`, if any.
    pub fn id(&self) -> Option<&str> {
        self.path.first().and_then(|e| e.id.as_deref())
    }
}

/// A hit map shared between the hit-test, input and script threads.
pub type SharedHitMap = Arc<Mutex<Arc<HitMap>>>;

/// All hit regions of the latest layout pass.
#[derive(Default)]
pub struct HitMap {
    regions: Vec<HitRegion>,
    /// Nodes whose cascaded CSS resolves `user-select: none`. Hit-map build
    /// sees the same styled arena used by layout, so the input loop can apply
    /// author selection policy without needing a cascaded DOM snapshot (the
    /// engine's live arena carries no `css_attr`).
    unselectable: HashSet<NodeId>,
    /// Native scrollbar controls keyed by their owning scroll container.
    pub scrollbars: HashMap<NodeId, ScrollbarRegion>,
    /// Hit-test index: region indices bucketed by canvas row (`rows[y]`
    /// lists every region whose rect covers row `y`), so a lookup inspects
    /// only the candidates on the pointer's row instead of scanning every
    /// region — the terminal's discrete rows are the natural spatial split.
    rows: Vec<Vec<u32>>,
}

impl HitMap {
    /// Create an empty hit map.
    pub fn new() -> HitMap {
        HitMap::default()
    }

    /// Number of collected regions.
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// True when no regions have been collected.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Iterate over the collected regions.
    pub fn iter(&self) -> impl Iterator<Item = &HitRegion> {
        self.regions.iter()
    }

    /// Build the region list by walking the layout forest in paint order.
    ///
    /// Every laid-out box becomes a region, so hit testing finds the element
    /// painted on top under the pointer (needed for CSS `:hover`, which
    /// applies to any element). Event dispatch is unaffected: the bubbling
    /// walk only invokes handlers for elements that declare `on*` attributes,
    /// and a non-interactive element's empty action map falls through to its
    /// ancestors.
    ///
    /// Three sharing passes keep the build proportional to unique content
    /// instead of boxes:
    ///
    /// - children are visited in the **paint stacking order** (the same
    ///   `z-index` / positioned-above-static key the paint phase sorts by),
    ///   so the region index doubles as the paint rank and hit testing can
    ///   resolve overlaps by "painted last wins" — the browser rule;
    /// - the bubbling path is resolved once per distinct node and shared
    ///   through an `Arc`;
    /// - the finished regions are bucketed by canvas row so
    ///   [`HitMap::hit_test`] never scans the whole list.
    pub fn build(geometry: &Geometry) -> HitMap {
        let mut regions = Vec::new();
        let mut scrollbars = HashMap::new();
        // Author selection policy: resolve `user-select` for every node while
        // the styled arena is at hand (the resolution walks the ancestor
        // chain, so a hit on any descendant of a `none` element is covered
        // and an explicit `text` child overrides its `none` ancestor).
        let mut unselectable = HashSet::new();
        for (node_id, _) in geometry.arena.iter() {
            if crate::layout::user_select_is_none(&geometry.arena, node_id) {
                unselectable.insert(node_id);
            }
        }
        // One ancestor-chain resolution per distinct node, shared by every
        // box the node produced.
        let mut paths: HashMap<NodeId, Arc<[ElementRef]>> = HashMap::new();
        let mut path_of = |node_id: NodeId| -> Arc<[ElementRef]> {
            paths
                .entry(node_id)
                .or_insert_with(|| ancestor_path(&geometry.arena, node_id).into())
                .clone()
        };
        collect_regions_roots(&geometry.boxes, &mut path_of, &mut regions, &mut scrollbars);
        // Inline `<label>` runs get their own small regions (the label's
        // text slice within its block box) so a click on label text resolves
        // to the label for `for=` / wrapped-control delegation. They are
        // appended after the box forest — they have no box of their own, so
        // they win against everything they overlap (an approximation: an
        // overlay painted above the label text would still lose the hit to
        // the label region).
        for (node_id, rect) in crate::layout::label_regions(&geometry.arena, &geometry.boxes) {
            regions.push(HitRegion {
                node_id,
                rect,
                client: rect,
                path: path_of(node_id),
            });
        }

        // Bucket the regions by canvas row for row-local hit tests.
        let mut rows: Vec<Vec<u32>> = Vec::new();
        for (idx, region) in regions.iter().enumerate() {
            let idx = idx as u32;
            let row_end = region.rect.y.saturating_add(region.rect.height);
            if rows.len() < row_end {
                rows.resize_with(row_end, Vec::new);
            }
            for bucket in &mut rows[region.rect.y..row_end] {
                bucket.push(idx);
            }
        }
        HitMap {
            regions,
            unselectable,
            scrollbars,
            rows,
        }
    }

    /// Whether `node_id`'s cascaded CSS resolves `user-select: none`.
    ///
    /// The set covers every node of the styled arena the map was built from
    /// (descendants of a `none` element included); an unknown id is
    /// selectable, matching a node that no longer exists.
    pub fn is_unselectable(&self, node_id: NodeId) -> bool {
        self.unselectable.contains(&node_id)
    }

    /// The topmost region containing the canvas point.
    ///
    /// Browser semantics: the hit target is the element **painted on top**.
    /// Regions are indexed in paint stacking order (see [`HitMap::build`]),
    /// so among the candidates covering the point the last one in paint
    /// order wins — a positioned overlay beats the static content beneath
    /// it, a higher `z-index` beats a lower one, and a child beats its
    /// ancestor. Only the regions covering the point's row are examined.
    pub fn hit_test(&self, x: u32, y: u32) -> Option<&HitRegion> {
        let mut best: Option<&HitRegion> = None;
        for &idx in self.rows.get(y as usize)? {
            let region = &self.regions[idx as usize];
            if region.rect.contains(x, y) {
                // Later region index = painted later = on top.
                best = Some(region);
            }
        }
        best
    }

    /// Look up a region by its stable `id`.
    ///
    /// Used to dispatch `MouseOut` when the pointer leaves a previously
    /// hovered element.
    pub fn resolve_by_id(&self, id: &str) -> Option<&HitRegion> {
        self.regions.iter().find(|r| r.id() == Some(id))
    }

    /// The first region collected for `node_id`, if any.
    ///
    /// Regions are indexed in paint stacking order, so for a node that laid
    /// out several boxes this is its topmost one — the same region
    /// [`HitMap::hit_test`] resolves a point inside it to.
    pub fn region_for(&self, node_id: NodeId) -> Option<&HitRegion> {
        self.regions.iter().find(|r| r.node_id == node_id)
    }

    /// Find the nearest scrollable ancestor (including `start`) whose native
    /// scrollbar is active.
    pub fn scroll_container(
        &self,
        start: Option<NodeId>,
        arena: &NodeArena,
    ) -> Option<&ScrollbarRegion> {
        let mut current = start;
        while let Some(node_id) = current {
            if let Some(scrollbar) = self.scrollbars.get(&node_id) {
                return Some(scrollbar);
            }
            current = arena.get(node_id).and_then(|node| node.parent);
        }
        None
    }
}

/// Walk the layout forest roots, pushing one region per box in paint
/// stacking order.
fn collect_regions_roots(
    roots: &[LayoutBox],
    path_of: &mut impl FnMut(NodeId) -> Arc<[ElementRef]>,
    out: &mut Vec<HitRegion>,
    scrollbars: &mut HashMap<NodeId, ScrollbarRegion>,
) {
    for root in roots {
        collect_regions(root, path_of, out, scrollbars);
    }
}

/// Push `box_tree`'s region, then recurse into its children in the paint
/// stacking order (the same key the paint phase sorts by), so the region
/// index doubles as the paint rank.
fn collect_regions(
    box_tree: &LayoutBox,
    path_of: &mut impl FnMut(NodeId) -> Arc<[ElementRef]>,
    out: &mut Vec<HitRegion>,
    scrollbars: &mut HashMap<NodeId, ScrollbarRegion>,
) {
    let rect = box_tree.rect;
    // Client box = border box inset by the box's own border widths.
    let border = &box_tree.paint_style.box_edges.border;
    let client = Rect {
        x: rect.x + border.left.width,
        y: rect.y + border.top.width,
        width: rect
            .width
            .saturating_sub(border.left.width + border.right.width),
        height: rect
            .height
            .saturating_sub(border.top.width + border.bottom.width),
    };
    out.push(HitRegion {
        node_id: box_tree.node_id,
        rect,
        client,
        path: path_of(box_tree.node_id),
    });
    if let Some(scroll) = box_tree.paint_style.scrollbar {
        scrollbars.insert(
            box_tree.node_id,
            ScrollbarRegion::new(box_tree.node_id, scroll),
        );
    }
    let mut children: Vec<&LayoutBox> = box_tree.children.iter().collect();
    children.sort_by_key(|c| crate::layout::paint::stacking_key(c));
    for child in children {
        collect_regions(child, path_of, out, scrollbars);
    }
}

/// Input-facing geometry for an engine-owned scrollbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarRegion {
    pub node_id: NodeId,
    pub track: Rect,
    pub thumb: Rect,
    pub scroll_y: u32,
    pub max_scroll: u32,
    /// Full scrollable content height in rows (`el.scrollHeight`).
    pub content_height: u32,
    /// Visible viewport height in rows (`el.clientHeight` of the scroller).
    pub viewport_height: u32,
    /// Current horizontal content offset in columns.
    pub scroll_x: u32,
    /// Maximum valid horizontal offset.
    pub max_scroll_x: u32,
    /// Full scrollable content width in columns (`el.scrollWidth`).
    pub content_width: u32,
    /// Visible viewport width in columns.
    pub viewport_width: u32,
    /// One-row bottom track rectangle (zero-height when there is no
    /// horizontal overflow).
    pub h_track: Rect,
    pub h_thumb: Rect,
}

impl ScrollbarRegion {
    fn new(node_id: NodeId, scroll: ScrollbarLayout) -> Self {
        Self {
            node_id,
            track: scroll.track,
            thumb: scroll.thumb,
            scroll_y: scroll.scroll_y as u32,
            max_scroll: scroll.max_scroll as u32,
            content_height: scroll.content_height as u32,
            viewport_height: scroll.viewport_height as u32,
            scroll_x: scroll.scroll_x as u32,
            max_scroll_x: scroll.max_scroll_x as u32,
            content_width: scroll.content_width as u32,
            viewport_width: scroll.viewport_width as u32,
            h_track: scroll.h_track,
            h_thumb: scroll.h_thumb,
        }
    }

    /// Content offset for a pointer column, preserving a thumb-grab offset
    /// (horizontal scrollbar drag).
    pub fn offset_at_x(&self, x: u32, grab_offset: u32) -> u32 {
        if self.h_track.width <= 1 || self.max_scroll_x == 0 {
            return 0;
        }
        let x = x as usize;
        let target = x
            .saturating_sub(grab_offset as usize)
            .saturating_sub(self.h_track.x)
            .min(self.h_track.width.saturating_sub(self.h_thumb.width.max(1)));
        let travel = self
            .h_track
            .width
            .saturating_sub(self.h_thumb.width.max(1))
            .max(1);
        ((target as u64 * self.max_scroll_x as u64) / travel as u64) as u32
    }

    /// Content offset for a pointer row, preserving a thumb-grab offset.
    pub fn offset_at(&self, y: u32, grab_offset: u32) -> u32 {
        if self.track.height <= 1 || self.max_scroll == 0 {
            return 0;
        }
        let y = y as usize;
        let target = y
            .saturating_sub(grab_offset as usize)
            .saturating_sub(self.track.y)
            .min(self.track.height.saturating_sub(self.thumb.height.max(1)));
        let travel = self
            .track
            .height
            .saturating_sub(self.thumb.height.max(1))
            .max(1);
        ((target as u64 * self.max_scroll as u64) / travel as u64) as u32
    }
}

/// Collect the DOM ancestor chain of `start`, target first.
fn ancestor_path(arena: &NodeArena, start: NodeId) -> Vec<ElementRef> {
    let mut path = Vec::new();
    let mut current = Some(start);
    while let Some(id) = current {
        let node = &arena[id];
        path.push(ElementRef {
            id: node.get_id().cloned(),
            node_id: id,
        });
        current = node.parent;
    }
    path
}

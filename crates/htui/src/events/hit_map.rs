//! Hit regions and spatial lookup over the laid-out box forest.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    layout::layoutbox::{LayoutBox, Rect},
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
        // One ancestor-chain resolution per distinct node, shared by every
        // box the node produced.
        let mut paths: HashMap<NodeId, Arc<[ElementRef]>> = HashMap::new();
        let mut path_of = |node_id: NodeId| -> Arc<[ElementRef]> {
            paths
                .entry(node_id)
                .or_insert_with(|| ancestor_path(&geometry.arena, node_id).into())
                .clone()
        };
        collect_regions_roots(&geometry.boxes, &mut path_of, &mut regions);
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
        HitMap { regions, rows }
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
}

/// Walk the layout forest roots, pushing one region per box in paint
/// stacking order.
fn collect_regions_roots(
    roots: &[LayoutBox],
    path_of: &mut impl FnMut(NodeId) -> Arc<[ElementRef]>,
    out: &mut Vec<HitRegion>,
) {
    for root in roots {
        collect_regions(root, path_of, out);
    }
}

/// Push `box_tree`'s region, then recurse into its children in the paint
/// stacking order (the same key the paint phase sorts by), so the region
/// index doubles as the paint rank.
fn collect_regions(
    box_tree: &LayoutBox,
    path_of: &mut impl FnMut(NodeId) -> Arc<[ElementRef]>,
    out: &mut Vec<HitRegion>,
) {
    out.push(HitRegion {
        node_id: box_tree.node_id,
        rect: box_tree.rect,
        path: path_of(box_tree.node_id),
    });
    let mut children: Vec<&LayoutBox> = box_tree.children.iter().collect();
    children.sort_by_key(|c| crate::layout::paint::stacking_key(c));
    for child in children {
        collect_regions(child, path_of, out);
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

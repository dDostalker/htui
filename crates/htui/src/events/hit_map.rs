//! Hit regions and spatial lookup over the laid-out box forest.

use std::{collections::HashMap, sync::Arc};

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

    /// Build the region list by walking the layout forest.
    ///
    /// Every laid-out box becomes a region, so hit testing finds the deepest
    /// element under the pointer (needed for CSS `:hover`, which applies to
    /// any element). Event dispatch is unaffected: the bubbling walk only
    /// invokes handlers for elements that declare `on*` attributes, and a
    /// non-interactive element's empty action map falls through to its
    /// ancestors.
    ///
    /// Two sharing passes keep the build proportional to unique content
    /// instead of boxes: the bubbling path is resolved once per distinct
    /// node and shared through an `Arc`, and the finished regions are
    /// bucketed by canvas row so [`HitMap::hit_test`] never scans the whole
    /// list.
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
        collect_regions(&geometry.boxes, &mut path_of, &mut regions);
        // Inline `<label>` runs get their own small regions (the label's
        // text slice within its block box) so a click on label text resolves
        // to the label for `for=` / wrapped-control delegation.
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

    /// Deepest region containing the canvas point.
    ///
    /// Children always sit inside their ancestors' content boxes, so the
    /// region with the smallest rect area is the innermost one; ties resolve
    /// to the later (deeper, in DFS order) candidate. Only the regions
    /// covering the point's row are examined.
    pub fn hit_test(&self, x: u32, y: u32) -> Option<&HitRegion> {
        let mut best: Option<&HitRegion> = None;
        for &idx in self.rows.get(y as usize)? {
            let region = &self.regions[idx as usize];
            if !region.rect.contains(x, y) {
                continue;
            }
            match best {
                None => best = Some(region),
                Some(current) => {
                    // Replace when the candidate is at least as specific
                    // (smaller area ⇒ deeper nesting). Equal areas keep the
                    // later, deeper DFS-order candidate.
                    if region.rect.area() <= current.rect.area() {
                        best = Some(region);
                    }
                }
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

fn collect_regions(
    boxes: &[LayoutBox],
    path_of: &mut impl FnMut(NodeId) -> Arc<[ElementRef]>,
    out: &mut Vec<HitRegion>,
) {
    for b in boxes {
        out.push(HitRegion {
            node_id: b.node_id,
            rect: b.rect,
            path: path_of(b.node_id),
        });
        collect_regions(&b.children, path_of, out);
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

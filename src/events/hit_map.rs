//! Hit regions and spatial lookup over the laid-out box forest.

use std::collections::HashMap;

use crate::{
    layout::layoutbox::{LayoutBox, Rect},
    tdom::node::{Node, NodeArena, NodeId},
};

use super::{ElementRef, EventType, Geometry};

/// A rectangular hit region for one laid-out element.
///
/// `rect` is in canvas coordinates (column/row space); `path` is the DOM
/// ancestor chain of the element (itself first, then parents), used for
/// bubbling dispatch.
///
/// Every layout box becomes a region so CSS `:hover` rules can target any
/// element; elements without `on*` attributes simply carry an empty `actions`
/// map and are skipped by the dispatcher.
#[derive(Debug, Clone)]
pub struct HitRegion {
    /// Session-local node id, used by hover enter / leave tracking to tell
    /// regions apart within one layout session. Not stable across `Redraw`s.
    pub node_id: NodeId,
    pub rect: Rect,
    /// `[self, parent, grandparent, …]` — the bubbling path.
    pub path: Vec<ElementRef>,
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
}

impl HitMap {
    pub fn new() -> HitMap {
        HitMap::default()
    }

    /// Number of collected regions.
    pub fn len(&self) -> usize {
        self.regions.len()
    }

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
    /// ancestors. Each region's bubbling path is resolved from the DOM
    /// `parent` pointers.
    pub fn build(geometry: &Geometry) -> HitMap {
        let mut regions = Vec::new();
        collect_regions(&geometry.boxes, &geometry.arena, &mut regions);
        HitMap { regions }
    }

    /// Deepest region containing the canvas point.
    ///
    /// Children always sit inside their ancestors' content boxes, so the
    /// region with the smallest rect area is the innermost one; ties resolve
    /// to the later (deeper, in DFS order) candidate.
    pub fn hit_test(&self, x: u32, y: u32) -> Option<&HitRegion> {
        let mut best: Option<&HitRegion> = None;
        for region in &self.regions {
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

fn collect_regions(boxes: &[LayoutBox], arena: &NodeArena, out: &mut Vec<HitRegion>) {
    for b in boxes {
        out.push(HitRegion {
            node_id: b.node_id,
            rect: b.rect,
            path: ancestor_path(arena, b.node_id),
        });
        collect_regions(&b.children, arena, out);
    }
}

/// Collect the `on*` event attributes of a node into an action map.
fn event_actions(node: &Node) -> HashMap<EventType, String> {
    let mut actions = HashMap::new();
    if let Some(value) = node.attributes.get("onclick") {
        actions.insert(EventType::Click, value.clone());
    }
    if let Some(value) = node.attributes.get("onmouseover") {
        actions.insert(EventType::MouseOver, value.clone());
    }
    if let Some(value) = node.attributes.get("onmouseout") {
        actions.insert(EventType::MouseOut, value.clone());
    }
    actions
}

/// Resolve the DOM ancestor chain of `start`, target first.
fn ancestor_path(arena: &NodeArena, start: NodeId) -> Vec<ElementRef> {
    let mut path = Vec::new();
    let mut current = Some(start);
    while let Some(id) = current {
        let node = &arena[id];
        path.push(ElementRef {
            id: node.get_id().cloned(),
            actions: event_actions(node),
        });
        current = node.parent;
    }
    path
}

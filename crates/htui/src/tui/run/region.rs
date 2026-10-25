//! Incremental region-repaint helpers: compute the smallest rectangle to
//! repaint for dirty nodes, pinned boxes and dynamic pseudo-class restyles.

use std::collections::HashMap;

use crate::html::css_parse::{
    CssRule, Position, PseudoClass, PseudoClassSet, selector_chain_matches_with_state,
};
use crate::layout::{LayoutBox, Rect};
use crate::tdom::node::{NodeArena, NodeId};

/// Compute the smallest rectangle whose repaint covers every change caused by
/// the dirty nodes, or `None` when the incremental path must be abandoned.
///
/// A DOM node paints inside the box of its nearest box-carrying ancestor
/// (inline elements fold into the enclosing block's runs), so each dirty node
/// is mapped up the DOM to the first ancestor that produced a `LayoutBox`.
/// That box must exist in both the cached and the fresh layout with an
/// identical rect — otherwise the change moved geometry (e.g. wrapping or a
/// structural mutation) and only a full repaint is safe. The union of the
/// matched boxes' rects is the repaint region.
///
/// Both layouts are indexed by node id once (`O(boxes)`), so each dirty
/// node's ancestor walk is a handful of hash lookups instead of a linear
/// scan of the whole flattened forest per candidate.
pub(super) fn incremental_region(
    old_boxes: &[LayoutBox],
    new_boxes: &[LayoutBox],
    arena: &NodeArena,
    dirty: &[NodeId],
) -> Option<Rect> {
    fn index(boxes: &[LayoutBox], out: &mut HashMap<NodeId, Vec<Rect>>) {
        for b in boxes {
            out.entry(b.node_id).or_default().push(b.rect);
            index(&b.children, out);
        }
    }
    let mut old_index: HashMap<NodeId, Vec<Rect>> = HashMap::new();
    let mut new_index: HashMap<NodeId, Vec<Rect>> = HashMap::new();
    index(old_boxes, &mut old_index);
    index(new_boxes, &mut new_index);

    let mut region: Option<Rect> = None;
    for &node_id in dirty {
        let mut found = false;
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            if let Some(rects) = new_index.get(&id) {
                if !rects.is_empty() {
                    found = true;
                    let old_rects = old_index.get(&id);
                    for rect in rects {
                        match old_rects {
                            Some(old) if old.contains(rect) => {
                                region = Some(match region {
                                    Some(r) => union_rect(r, *rect),
                                    None => *rect,
                                });
                            }
                            _ => return None,
                        }
                    }
                    break;
                }
            }
            cur = arena.get(id).and_then(|n| n.parent);
        }
        if !found {
            return None;
        }
    }
    region
}

/// Union of two canvas rectangles (absolute coordinates).
pub(super) fn union_rect(a: Rect, b: Rect) -> Rect {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = a.x.saturating_add(a.width).max(b.x.saturating_add(b.width));
    let bottom =
        a.y.saturating_add(a.height)
            .max(b.y.saturating_add(b.height));
    Rect::new(left, top, right - left, bottom - top)
}

/// First box rect belonging to `node_id`, in tree order.
///
/// Single-target lookup used by the input-repaint paths (keystroke / select
/// open / close): walking the tree with an early return avoids materializing
/// the full `(NodeId, Rect)` flat list just to `find` one entry.
pub(super) fn find_box_rect(boxes: &[LayoutBox], node_id: NodeId) -> Option<Rect> {
    fn walk(boxes: &[LayoutBox], node_id: NodeId) -> Option<Rect> {
        for b in boxes {
            if b.node_id == node_id {
                return Some(b.rect);
            }
            if let Some(found) = walk(&b.children, node_id) {
                return Some(found);
            }
        }
        None
    }
    walk(boxes, node_id)
}

/// Every `position: fixed` / `sticky` box's rect (document coordinates), in
/// tree order.
///
/// Used to compute the repaint region when pinned boxes are re-anchored to a
/// new scroll offset, and to detect whether the document has any pinned box
/// (the scroll-aware repaint only runs when it does).
pub(super) fn pinned_boxes_rects(boxes: &[LayoutBox]) -> Vec<Rect> {
    fn walk(boxes: &[LayoutBox], out: &mut Vec<Rect>) {
        for b in boxes {
            if matches!(b.positioning.position, Position::Fixed | Position::Sticky) {
                out.push(b.rect);
            }
            walk(&b.children, out);
        }
    }
    let mut out = Vec::new();
    walk(boxes, &mut out);
    out
}

/// Union of every box rect that a dynamic pseudo-class's state target being
/// `target` can change: the target plus (for `:hover`) each ancestor that
/// matches a rule of that pseudo-class. Candidates that match no rule are
/// skipped — otherwise hovering a bottom-of-page button would repaint the
/// whole document through its `body` ancestor.
///
/// Used by `repaint_dynamic` (backing the `Hover` / `Focus` commands) to
/// repaint only the affected region instead of the whole canvas.
pub(super) fn pseudo_affected_region(
    boxes: &[LayoutBox],
    arena: &NodeArena,
    rules: &CssRule,
    pseudo: PseudoClass,
    target: Option<NodeId>,
) -> Option<Rect> {
    let match_state = pseudo.state_with_target(target);
    let mut region: Option<Rect> = None;
    for id in pseudo.affected_candidates(arena, target) {
        // A stale target — the element JS removed between the hit map's
        // snapshot and this repaint (the quick-click race) — can contribute
        // nothing; skip it instead of matching against a missing node.
        if arena.get(id).is_none() {
            continue;
        }
        // Only a selector that contains `pseudo` and matches `id` while the
        // state target is `target` can repaint `id`'s boxes.
        let affected = rules.0.iter().any(|entry| {
            let sel = &entry.selector;
            PseudoClassSet::from_selector(sel).contains_pseudo(pseudo)
                && selector_chain_matches_with_state(arena, id, sel, &match_state)
        });
        if affected {
            let mut stack = vec![boxes];
            while let Some(list) = stack.pop() {
                for b in list {
                    if b.node_id == id {
                        region = Some(match region {
                            Some(r) => union_rect(r, b.rect),
                            None => b.rect,
                        });
                    }
                    stack.push(&b.children);
                }
            }
        }
    }
    region
}

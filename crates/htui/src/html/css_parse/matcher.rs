use crate::tdom::node::{NodeArena, NodeId};

use super::selector::{Combinator, CompoundSelector, CssSelector, NthExpr, PseudoClass, Selector};

fn matches_css(arena: &NodeArena, node_id: NodeId, sel: &Selector, hovered: Option<NodeId>) -> bool {
    let node = &arena[node_id];
    match sel {
        Selector::Tag(nt) => node.node_type == *nt,
        Selector::Class(name) => node.has_class(name),
        Selector::Id(id) => node
            .get_id()
            .map(|s| s.as_str() == id.as_str())
            .unwrap_or(false),
        Selector::NthChild(expr) => nth_child_matches(arena, node_id, *expr),
        Selector::PseudoClass(PseudoClass::Hover) => hovered.is_some_and(|h| is_hovered_or_ancestor(arena, node_id, h)),
    }
}

fn matches_compound(
    arena: &NodeArena,
    node_id: NodeId,
    compound: &CompoundSelector,
    hovered: Option<NodeId>,
) -> bool {
    compound
        .iter()
        .all(|s| matches_css(arena, node_id, s, hovered))
}

/// Compute the 1-based sibling index of `node_id` among its parent's
/// children. Text nodes count toward the index (matching the CSS spec) so
/// that `<div>text<p>...</p></div>` reports `<p>` as index 2, not 1.
fn nth_child_matches(arena: &NodeArena, node_id: NodeId, expr: NthExpr) -> bool {
    let Some(parent_id) = arena[node_id].parent else {
        // Detached node — `:nth-child` is meaningless without a sibling
        // list, so decline to match.
        return false;
    };
    let parent = &arena[parent_id];
    let mut index = 1usize;
    for &cid in &parent.children {
        if cid == node_id {
            return expr.matches(index);
        }
        index = index.saturating_add(1);
    }
    // `node_id` not listed as a child — should not happen for a well-formed
    // DOM, but treat as no-match rather than panicking.
    false
}

/// Check whether a node matches a CSS selector chain, with no `:hover`
/// context (i.e. `:hover` rules never match).
#[doc(hidden)]
pub fn selector_chain_matches(arena: &NodeArena, node_id: NodeId, selector: &CssSelector) -> bool {
    selector_chain_matches_with_hover(arena, node_id, selector, None)
}

/// Like [`selector_chain_matches`], but evaluates `:hover` against the node
/// currently under the pointer.
///
/// `hovered` is a session-local [`NodeId`]; a stale id from a previous layout
/// session simply fails to match (it is looked up via `arena.get`, never
/// indexed directly).
pub fn selector_chain_matches_with_hover(
    arena: &NodeArena,
    node_id: NodeId,
    selector: &CssSelector,
    hovered: Option<NodeId>,
) -> bool {
    let n = selector.compounds.len();
    if n == 0 {
        return false;
    }
    if !matches_compound(arena, node_id, &selector.compounds[n - 1], hovered) {
        return false;
    }
    if n == 1 {
        return true;
    }
    match_combinators(
        arena,
        node_id,
        &selector.compounds,
        &selector.combinators,
        n - 1,
        hovered,
    )
}

/// True when `node_id` is the hovered element or one of its ancestors — the
/// CSS `:hover` semantics (the pointer being over a descendant counts).
fn is_hovered_or_ancestor(arena: &NodeArena, node_id: NodeId, hovered: NodeId) -> bool {
    let mut current = Some(hovered);
    while let Some(id) = current {
        if id == node_id {
            return true;
        }
        current = arena.get(id).and_then(|n| n.parent);
    }
    false
}

fn match_combinators(
    arena: &NodeArena,
    node_id: NodeId,
    compounds: &[CompoundSelector],
    combinators: &[Combinator],
    idx: usize,
    hovered: Option<NodeId>,
) -> bool {
    if idx == 0 {
        return true;
    }
    let combinator = combinators[idx - 1];
    let target = &compounds[idx - 1];

    match combinator {
        Combinator::Descendant => {
            let mut cur = arena[node_id].parent;
            while let Some(pid) = cur {
                let ancestor = &arena[pid];
                if matches_compound(arena, pid, target, hovered)
                    && match_combinators(arena, pid, compounds, combinators, idx - 1, hovered)
                {
                    return true;
                }
                cur = ancestor.parent;
            }
            false
        }
        Combinator::Child => match arena[node_id].parent {
            Some(pid) if matches_compound(arena, pid, target, hovered) => {
                match_combinators(arena, pid, compounds, combinators, idx - 1, hovered)
            }
            _ => false,
        },
        Combinator::NextSibling => match prev_sibling_id(arena, node_id) {
            Some(sid) if matches_compound(arena, sid, target, hovered) => {
                match_combinators(arena, sid, compounds, combinators, idx - 1, hovered)
            }
            _ => false,
        },
        Combinator::SubsequentSibling => {
            let mut cur = prev_sibling_id(arena, node_id);
            while let Some(sid) = cur {
                if matches_compound(arena, sid, target, hovered)
                    && match_combinators(arena, sid, compounds, combinators, idx - 1, hovered)
                {
                    return true;
                }
                cur = prev_sibling_id(arena, sid);
            }
            false
        }
    }
}

fn prev_sibling_id(arena: &NodeArena, node_id: NodeId) -> Option<NodeId> {
    let node = &arena[node_id];
    let parent_id = node.parent?;
    let parent = &arena[parent_id];
    let mut prev = None;
    for &cid in &parent.children {
        if cid == node_id {
            return prev;
        }
        prev = Some(cid);
    }
    None
}

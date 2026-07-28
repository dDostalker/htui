use crate::tdom::node::{Node, NodeArena, NodeId};

use super::selector::{Combinator, CompoundSelector, CssSelector, Selector};

fn matches_css(node: &Node, sel: &Selector) -> bool {
    match sel {
        Selector::Tag(nt) => node.node_type == *nt,
        Selector::Class(name) => node.has_class(name),
        Selector::Id(id) => node
            .get_id()
            .map(|s| s.as_str() == id.as_str())
            .unwrap_or(false),
    }
}

fn matches_compound(node: &Node, compound: &CompoundSelector) -> bool {
    compound.iter().all(|s| matches_css(node, s))
}

/// Check whether a node matches a CSS selector chain.
#[doc(hidden)]
pub fn selector_chain_matches(arena: &NodeArena, node_id: NodeId, selector: &CssSelector) -> bool {
    let node = &arena[node_id];
    let n = selector.compounds.len();
    if n == 0 {
        return false;
    }
    if !matches_compound(node, &selector.compounds[n - 1]) {
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
    )
}

fn match_combinators(
    arena: &NodeArena,
    node_id: NodeId,
    compounds: &[CompoundSelector],
    combinators: &[Combinator],
    idx: usize,
) -> bool {
    if idx == 0 {
        return true;
    }
    let combinator = combinators[idx - 1];
    let target = &compounds[idx - 1];
    let node = &arena[node_id];

    match combinator {
        Combinator::Descendant => {
            let mut cur = node.parent;
            while let Some(pid) = cur {
                let ancestor = &arena[pid];
                if matches_compound(ancestor, target)
                    && match_combinators(arena, pid, compounds, combinators, idx - 1)
                {
                    return true;
                }
                cur = ancestor.parent;
            }
            false
        }
        Combinator::Child => match node.parent {
            Some(pid) if matches_compound(&arena[pid], target) => {
                match_combinators(arena, pid, compounds, combinators, idx - 1)
            }
            _ => false,
        },
        Combinator::NextSibling => match prev_sibling_id(arena, node_id) {
            Some(sid) if matches_compound(&arena[sid], target) => {
                match_combinators(arena, sid, compounds, combinators, idx - 1)
            }
            _ => false,
        },
        Combinator::SubsequentSibling => {
            let mut cur = prev_sibling_id(arena, node_id);
            while let Some(sid) = cur {
                if matches_compound(&arena[sid], target)
                    && match_combinators(arena, sid, compounds, combinators, idx - 1)
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

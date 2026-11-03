use crate::tdom::node::{NodeArena, NodeId, NodeType};

use super::selector::{
    Combinator, CompoundSelector, CssSelector, DynamicState, NthExpr, PseudoClass, Selector,
};

impl PseudoClass {
    /// The matching semantics of a dynamic pseudo-class: read the slot of
    /// `state` this pseudo-class owns and judge `node_id` by its DOM rule.
    /// `:hover` follows the ancestor chain (the pointer being over a
    /// descendant counts); `:focus` matches the focused element itself.
    pub fn matches(&self, arena: &NodeArena, node_id: NodeId, state: &DynamicState) -> bool {
        match self {
            PseudoClass::Hover => state
                .hovered
                .is_some_and(|h| is_hovered_or_ancestor(arena, node_id, h)),
            PseudoClass::Focus => state.focused == Some(node_id),
            PseudoClass::FocusVisible => state.focused == Some(node_id),
        }
    }

    /// The nodes whose painted output can change when this pseudo-class's
    /// state target moves to `target`: the target plus (for `:hover`) every
    /// ancestor that `:hover` rules can match. `:focus` is exact — only the
    /// focused element's own boxes can be restyled.
    pub fn affected_candidates(&self, arena: &NodeArena, target: Option<NodeId>) -> Vec<NodeId> {
        match self {
            PseudoClass::Hover => ancestor_chain(arena, target),
            PseudoClass::Focus | PseudoClass::FocusVisible => target.into_iter().collect(),
        }
    }

    /// A [`DynamicState`] with this pseudo-class's slot set to `target` and
    /// every other slot cleared — the match context for deciding whether a
    /// rule is affected by a state change of this pseudo-class alone.
    pub fn state_with_target(&self, target: Option<NodeId>) -> DynamicState {
        match self {
            PseudoClass::Hover => DynamicState {
                hovered: target,
                ..Default::default()
            },
            PseudoClass::Focus | PseudoClass::FocusVisible => DynamicState {
                focused: target,
                ..Default::default()
            },
        }
    }
}

/// `target` and its ancestors, target-first, ending at the root (or at the
/// root-most reachable node for a detached node).
fn ancestor_chain(arena: &NodeArena, target: Option<NodeId>) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut cur = target;
    while let Some(id) = cur {
        out.push(id);
        cur = arena.get(id).and_then(|n| n.parent);
    }
    out
}

fn matches_css(arena: &NodeArena, node_id: NodeId, sel: &Selector, state: &DynamicState) -> bool {
    // A stale id from a previous arena generation — for example, a node JS
    // removed after the hit-map snapshot — must fail to match, not panic. The public
    // entry points' doc contract promises `arena.get` lookups everywhere.
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    match sel {
        // `*` matches every element node, never non-element nodes (text,
        // comment, document, doctype, document-fragment).
        Selector::Universal => !matches!(
            node.node_type,
            NodeType::Document
                | NodeType::DocumentType
                | NodeType::DocumentFragment
                | NodeType::Text
                | NodeType::Comment
        ),
        Selector::Tag(nt) => {
            node.node_type == *nt && !(node.is_custom_element() && *nt == NodeType::Span)
        }
        Selector::TagName(name) => node.tag_name().is_some_and(|tag| tag == name.as_str()),
        Selector::Class(name) => node.has_class(name),
        Selector::Id(id) => node
            .get_id()
            .map(|s| s.as_str() == id.as_str())
            .unwrap_or(false),
        Selector::NthChild(expr) => nth_child_matches(arena, node_id, *expr),
        Selector::NthLastChild(expr) => nth_last_child_matches(arena, node_id, *expr),
        Selector::NthOfType(expr) => nth_of_type_matches(arena, node_id, *expr, false),
        Selector::NthLastOfType(expr) => nth_of_type_matches(arena, node_id, *expr, true),
        Selector::Root => is_root_element(arena, node_id),
        Selector::Empty => is_empty_element(arena, node_id),
        Selector::Not(selectors) => !selectors
            .iter()
            .any(|selector| selector_chain_matches_with_state(arena, node_id, selector, state)),
        Selector::Is(selectors) | Selector::Where(selectors) => selectors
            .iter()
            .any(|selector| selector_chain_matches_with_state(arena, node_id, selector, state)),
        Selector::Has(arguments) => has_matches(arena, node_id, arguments, state),
        Selector::Checked => checked_matches(arena, node_id),
        Selector::Enabled => enabled_matches(arena, node_id),
        Selector::Disabled => disabled_matches(arena, node_id),
        Selector::Lang(tag) => lang_matches(arena, node_id, tag),
        Selector::PseudoClass(p) => p.matches(arena, node_id, state),
        Selector::Attribute(attr) => {
            let node2 = arena.get(node_id);
            let node2 = match node2 {
                Some(n) => n,
                None => return false,
            };
            attr.matches_lookup(&|name| node2.attributes.get(name).cloned())
        }
    }
}

fn matches_compound(
    arena: &NodeArena,
    node_id: NodeId,
    compound: &CompoundSelector,
    state: &DynamicState,
) -> bool {
    compound
        .iter()
        .all(|s| matches_css(arena, node_id, s, state))
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

/// Compute the 1-based sibling index of `node_id` counting from the end of
/// its parent's children (the last child is index 1). Like
/// [`nth_child_matches`], text nodes count toward the index (CSS spec).
fn nth_last_child_matches(arena: &NodeArena, node_id: NodeId, expr: NthExpr) -> bool {
    let Some(parent_id) = arena.get(node_id).and_then(|n| n.parent) else {
        // Detached node — no sibling list, decline to match.
        return false;
    };
    let parent = &arena[parent_id];
    let Some(pos) = parent.children.iter().position(|&c| c == node_id) else {
        return false;
    };
    expr.matches(parent.children.len() - pos)
}

/// The `*-of-type` family (`:nth-of-type` / `:nth-last-of-type` and their
/// index-1 decompositions): count only siblings with the same element type
/// as `node_id` — text / comment children never join the list (the CSS "of
/// its type" semantics). `from_end` counts 1-based from the last same-type
/// sibling instead of the first.
fn nth_of_type_matches(arena: &NodeArena, node_id: NodeId, expr: NthExpr, from_end: bool) -> bool {
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    let Some(parent_id) = node.parent else {
        // Detached node — no sibling list, decline to match.
        return false;
    };
    let node_type = node.node_type;
    let parent = &arena[parent_id];
    let mut position = None;
    let mut typed_count = 0usize;
    for &child in &parent.children {
        if arena.get(child).is_some_and(|n| n.node_type == node_type) {
            if child == node_id {
                position = Some(typed_count);
            }
            typed_count += 1;
        }
    }
    let Some(pos) = position else {
        return false;
    };
    let index = if from_end { typed_count - pos } else { pos + 1 };
    expr.matches(index)
}

/// `:root` — the element whose parent is the `Document` node (`<html>` in a
/// parsed document; an HTML fragment parsed into a fragment has no root).
fn is_root_element(arena: &NodeArena, node_id: NodeId) -> bool {
    arena
        .get(node_id)
        .and_then(|n| n.parent)
        .and_then(|pid| arena.get(pid))
        .is_some_and(|p| p.node_type == NodeType::Document)
}

/// `:empty` — no children that affect emptiness: element children always do,
/// text children do when their data is non-empty (whitespace counts), and
/// comments never do (CSS Selectors semantics, matching browsers).
fn is_empty_element(arena: &NodeArena, node_id: NodeId) -> bool {
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    node.children.iter().all(|&c| match arena.get(c) {
        Some(child) => match child.node_type {
            NodeType::Comment => true,
            NodeType::Text => child.data.as_deref().is_none_or(str::is_empty),
            _ => false,
        },
        // A stale child id (node freed outside this walk) is conservative
        // evidence of content, same as any other element child.
        None => false,
    })
}

/// `:checked` — a checked `<input type=checkbox|radio>` (the engine toggles
/// the `checked` attribute on click) or a `<option selected>`.
fn checked_matches(arena: &NodeArena, node_id: NodeId) -> bool {
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    match node.node_type {
        NodeType::Input => {
            let toggle = matches!(
                node.attributes.get("type").map(String::as_str),
                Some("checkbox" | "radio")
            );
            toggle && node.attributes.contains_key("checked")
        }
        NodeType::HtmlOption => node.attributes.contains_key("selected"),
        _ => false,
    }
}

/// `:disabled` — a form control carrying its own `disabled` attribute, or a
/// descendant of a `<fieldset disabled>`. The spec's exemption for the first
/// `<legend>`'s descendants is not modelled.
fn disabled_matches(arena: &NodeArena, node_id: NodeId) -> bool {
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    if is_form_control(node.node_type) && node.attributes.contains_key("disabled") {
        return true;
    }
    let mut cur = node.parent;
    while let Some(id) = cur {
        let Some(n) = arena.get(id) else {
            break;
        };
        if n.node_type == NodeType::Fieldset && n.attributes.contains_key("disabled") {
            return true;
        }
        cur = n.parent;
    }
    false
}

/// `:enabled` — a form control that is not `:disabled`. Non-form-control
/// elements match neither `:enabled` nor `:disabled` (CSS Selectors spec).
fn enabled_matches(arena: &NodeArena, node_id: NodeId) -> bool {
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    is_form_control(node.node_type) && !disabled_matches(arena, node_id)
}

/// `:lang(<tag>)` — the nearest `lang` attribute on the element or an
/// ancestor declares a language whose primary subtag matches `tag`
/// (ASCII case-insensitive; `:lang(zh)` matches `lang="zh-CN"`).
fn lang_matches(arena: &NodeArena, node_id: NodeId, tag: &str) -> bool {
    let mut cur = Some(node_id);
    while let Some(id) = cur
        && let Some(node) = arena.get(id)
    {
        if let Some(declared) = node.attributes.get("lang") {
            let declared = declared.trim().to_ascii_lowercase();
            let primary = declared.split(['-', '_']).next().unwrap_or_default();
            return primary == tag;
        }
        cur = node.parent;
    }
    false
}

/// The element types HTML marks as disable-able (the `:enabled` / `:disabled`
/// domain): input, textarea, select, button, optgroup, option and fieldset.
fn is_form_control(node_type: NodeType) -> bool {
    matches!(
        node_type,
        NodeType::Input
            | NodeType::Textarea
            | NodeType::Select
            | NodeType::Button
            | NodeType::Optgroup
            | NodeType::HtmlOption
            | NodeType::Fieldset
    )
}

/// A `:has()` argument matches when one of its anchored candidates satisfies
/// the wrapped complex selector. For example, `:has(> footer .tag)` anchors at
/// a child `footer` and then requires a descendant `.tag` below that footer.
fn has_matches(
    arena: &NodeArena,
    candidate: NodeId,
    arguments: &[super::selector::RelativeSelector],
    state: &DynamicState,
) -> bool {
    arguments.iter().any(|argument| {
        let compounds = &argument.selector.compounds;
        if compounds.is_empty() {
            return false;
        }
        relative_anchor_ids(arena, candidate, argument.anchor)
            .into_iter()
            .any(|anchor| {
                matches_compound(arena, anchor, &compounds[0], state)
                    && has_forward_matches(
                        arena,
                        anchor,
                        compounds,
                        &argument.selector.combinators,
                        0,
                        state,
                    )
            })
    })
}

/// Match the remainder of a `:has()` argument left-to-right after its first
/// compound has already been found. `:has(> footer .tag)` first anchors at a
/// child `footer`, then this function looks below it for `.tag`.
fn has_forward_matches(
    arena: &NodeArena,
    current: NodeId,
    compounds: &[CompoundSelector],
    combinators: &[Combinator],
    idx: usize,
    state: &DynamicState,
) -> bool {
    if idx == combinators.len() {
        return true;
    }
    let combinator = combinators[idx];
    let target = &compounds[idx + 1];
    let candidates = match combinator {
        Combinator::Descendant => relative_anchor_ids(arena, current, Combinator::Descendant),
        Combinator::Child => relative_anchor_ids(arena, current, Combinator::Child),
        Combinator::NextSibling => next_sibling_id(arena, current).into_iter().collect(),
        Combinator::SubsequentSibling => sibling_ids_after(arena, current).unwrap_or_default(),
    };
    candidates.into_iter().any(|next| {
        matches_compound(arena, next, target, state)
            && has_forward_matches(arena, next, compounds, combinators, idx + 1, state)
    })
}

/// Element siblings after `node_id` in document order.
fn sibling_ids_after(arena: &NodeArena, node_id: NodeId) -> Option<Vec<NodeId>> {
    let node = arena.get(node_id)?;
    let parent_id = node.parent?;
    let parent = arena.get(parent_id)?;
    let pos = parent.children.iter().position(|&cid| cid == node_id)?;
    Some(parent.children[pos + 1..].to_vec())
}

fn next_sibling_id(arena: &NodeArena, node_id: NodeId) -> Option<NodeId> {
    let node = arena.get(node_id)?;
    let parent_id = node.parent?;
    let parent = arena.get(parent_id)?;
    let pos = parent.children.iter().position(|&cid| cid == node_id)?;
    parent.children.get(pos + 1).copied()
}

/// Return the elements that can serve as anchors for a relative `:has()`
/// selector. Specific tags/compound predicates later reject non-element ids,
/// so this traversal can simply follow the DOM child/sibling structure.
fn relative_anchor_ids(arena: &NodeArena, candidate: NodeId, anchor: Combinator) -> Vec<NodeId> {
    match anchor {
        Combinator::Descendant => {
            let mut out = Vec::new();
            let mut stack = arena
                .get(candidate)
                .map(|node| node.children.clone())
                .unwrap_or_default();
            while let Some(id) = stack.pop() {
                if let Some(node) = arena.get(id) {
                    out.push(id);
                    stack.extend(node.children.iter().rev().copied());
                }
            }
            out
        }
        Combinator::Child => arena
            .get(candidate)
            .map(|node| node.children.clone())
            .unwrap_or_default(),
        // In a normal selector, `+` / `~` point at preceding siblings, but
        // as `:has()` anchors they describe siblings **after** the candidate.
        Combinator::NextSibling => next_sibling_id(arena, candidate).into_iter().collect(),
        Combinator::SubsequentSibling => sibling_ids_after(arena, candidate).unwrap_or_default(),
    }
}

/// Check whether a node matches a CSS selector chain, with no `:hover`
/// context (i.e. `:hover` rules never match).
#[doc(hidden)]
pub fn selector_chain_matches(arena: &NodeArena, node_id: NodeId, selector: &CssSelector) -> bool {
    selector_chain_matches_with_hover(arena, node_id, selector, None)
}

/// Like [`selector_chain_matches`], but evaluates the dynamic pseudo-classes
/// against a session [`DynamicState`] snapshot.
///
/// A stale node id from a previous layout session simply fails to match (it
/// is looked up via `arena.get`, never indexed directly).
pub fn selector_chain_matches_with_state(
    arena: &NodeArena,
    node_id: NodeId,
    selector: &CssSelector,
    state: &DynamicState,
) -> bool {
    let n = selector.compounds.len();
    if n == 0 {
        return false;
    }
    if !matches_compound(arena, node_id, &selector.compounds[n - 1], state) {
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
        state,
    )
}

/// Like [`selector_chain_matches`], but evaluates `:hover` against the node
/// currently under the pointer (`:focus` never matches).
pub fn selector_chain_matches_with_hover(
    arena: &NodeArena,
    node_id: NodeId,
    selector: &CssSelector,
    hovered: Option<NodeId>,
) -> bool {
    selector_chain_matches_with_state(
        arena,
        node_id,
        selector,
        &DynamicState {
            hovered,
            ..Default::default()
        },
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
    state: &DynamicState,
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
                if matches_compound(arena, pid, target, state)
                    && match_combinators(arena, pid, compounds, combinators, idx - 1, state)
                {
                    return true;
                }
                cur = ancestor.parent;
            }
            false
        }
        Combinator::Child => match arena[node_id].parent {
            Some(pid) if matches_compound(arena, pid, target, state) => {
                match_combinators(arena, pid, compounds, combinators, idx - 1, state)
            }
            _ => false,
        },
        Combinator::NextSibling => match prev_sibling_id(arena, node_id) {
            Some(sid) if matches_compound(arena, sid, target, state) => {
                match_combinators(arena, sid, compounds, combinators, idx - 1, state)
            }
            _ => false,
        },
        Combinator::SubsequentSibling => {
            let Some(siblings) = sibling_ids_before(arena, node_id) else {
                return false;
            };
            for &sid in siblings.iter().rev() {
                if matches_compound(arena, sid, target, state)
                    && match_combinators(arena, sid, compounds, combinators, idx - 1, state)
                {
                    return true;
                }
            }
            false
        }
    }
}

fn sibling_ids_before(arena: &NodeArena, node_id: NodeId) -> Option<&[NodeId]> {
    let node = arena.get(node_id)?;
    let parent_id = node.parent?;
    let parent = arena.get(parent_id)?;
    let pos = parent.children.iter().position(|&cid| cid == node_id)?;
    Some(&parent.children[..pos])
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

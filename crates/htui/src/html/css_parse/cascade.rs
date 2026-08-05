use indexmap::IndexMap;

use crate::tdom::node::{NodeArena, NodeId};

use super::{
    matcher::selector_chain_matches_with_hover, properties::CssAttr, selector::CssSelector,
    stylesheet::{CssRule, parse_declaration_text},
};

fn apply_css_rule(
    arena: &mut NodeArena,
    node_id: NodeId,
    selector: &CssSelector,
    css: &CssAttr,
    hovered: Option<NodeId>,
) {
    if selector_chain_matches_with_hover(arena, node_id, selector, hovered) {
        arena[node_id].css_attr.add(css.clone());
    }
    let children: Vec<NodeId> = arena[node_id].children.clone();
    for child_id in children {
        apply_css_rule(arena, child_id, selector, css, hovered);
    }
}
fn apply_inline_styles(arena: &mut NodeArena, node_id: NodeId) {
    let inline_style = arena[node_id].attributes.get("style").cloned();
    if let Some(style) = inline_style {
        arena[node_id]
            .css_attr
            .add(CssAttr(parse_declaration_text(&style)));
    }

    let children = arena[node_id].children.clone();
    for child_id in children {
        apply_inline_styles(arena, child_id);
    }
}

// css_into_node

/// Apply all stylesheet rules, inline `style` attributes and inheritance to
/// the arena, evaluating `:hover` pseudo-class rules against `hovered` (the
/// session-local node currently under the pointer).
///
/// Callers that need a hover-aware cascade (the event system's hover
/// highlight) use this; everything else should call [`css_into_node`], which
/// cascades without any `:hover` matching.
pub fn css_into_node_with_hover(
    arena: &mut NodeArena,
    root_id: NodeId,
    css: &CssRule,
    hovered: Option<NodeId>,
) {
    for (selector, attr) in css.0.iter() {
        apply_css_rule(arena, root_id, selector, attr, hovered);
    }
    // Inline declarations have higher author-level precedence than matching
    // stylesheet rules and must be present before inheritance is resolved.
    apply_inline_styles(arena, root_id);
    // Propagate inheritable CSS properties (color, text-align, font-weight,
    // font-style, text-decoration)
    inherit_css(arena, root_id, &IndexMap::new());
}

/// Cascade styles without a hover context (no `:hover` rules match).
pub fn css_into_node(arena: &mut NodeArena, root_id: NodeId, css: &CssRule) {
    css_into_node_with_hover(arena, root_id, css, None);
}

/// Clear the parsed CSS attributes of every node in the subtree rooted at
/// `root_id`, preparing the arena for a fresh [`css_into_node_with_hover`]
/// pass (the cascade only ever adds, so re-running it requires a reset).
pub fn reset_css_attr(arena: &mut NodeArena, root_id: NodeId) {
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        arena[id].css_attr = CssAttr::new();
        stack.extend(arena[id].children.iter().copied());
    }
}

/// All CSS properties the renderer recognises by name.
///
/// Single source of truth for property names. Adding support for a new property:
///   1. Append the name here.
///   2. Add an accessor on [`CssAttr`] (e.g. `get_padding()`).
///   3. If it should inherit, also append to [`INHERITED_PROPS`].
///
/// No other site needs editing — the cascade in [`inherit_css`] drives itself
/// from these lists.
pub const SUPPORTED_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "background",
    "text-align",
    "font-weight",
    "font-style",
    "text-decoration",
    "font-size",
    "width",
    "height",
    // High-frequency sizing & box-model semantics
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "box-sizing",
    "border-radius",
    // Text
    "text-transform",
    "line-height",
    // Box-model edges
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-width",
    "border-style",
    "border-color",
    // Flexbox
    "display",
    "flex-direction",
    "flex-wrap",
    "justify-content",
    "align-items",
    "align-self",
    "align-content",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "order",
    "gap",
    "row-gap",
    "column-gap",
    // Text wrapping (CSS Text Level 3)
    "white-space",
    "word-break",
    "overflow-wrap",
    "word-wrap",
];

/// Inheritable subset of [`SUPPORTED_PROPERTIES`]: a child without an explicit
/// value picks up its parent's.
///
/// Kept as a separate list because Rust const-fn filtering isn't yet stable;
/// appending a name here is what flips a property to inheritable.
pub const INHERITED_PROPS: &[&str] = &[
    "color",
    "text-align",
    "font-weight",
    "font-style",
    "text-decoration",
    "font-size",
    "text-transform",
    "line-height",
];

fn inherit_css(arena: &mut NodeArena, node_id: NodeId, parent_attrs: &IndexMap<String, String>) {
    // Merge parent inheritable attrs into current node (if not already set)
    let node = &mut arena[node_id];
    for key in INHERITED_PROPS {
        if let Some(value) = parent_attrs.get(*key) {
            node.css_attr
                .0
                .entry((*key).to_string())
                .or_insert_with(|| value.clone());
        }
    }

    // Collect inheritable attrs from this node to pass down
    let mut merged = parent_attrs.clone();
    for key in INHERITED_PROPS {
        if let Some(value) = node.css_attr.0.get(*key) {
            merged.insert((*key).to_string(), value.clone());
        }
    }

    // Recurse into children (clone to avoid borrow issues)
    let children: Vec<NodeId> = node.children.clone();
    for child_id in children {
        inherit_css(arena, child_id, &merged);
    }
}

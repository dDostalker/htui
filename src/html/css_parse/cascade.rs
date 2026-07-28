use indexmap::IndexMap;

use crate::tdom::node::{NodeArena, NodeId};

use super::{
    matcher::selector_chain_matches, properties::CssAttr, selector::CssSelector,
    stylesheet::CssRule,
};

fn apply_css_rule(arena: &mut NodeArena, node_id: NodeId, selector: &CssSelector, css: &CssAttr) {
    if selector_chain_matches(arena, node_id, selector) {
        arena[node_id].css_attr.add(css.clone());
    }
    let children: Vec<NodeId> = arena[node_id].children.clone();
    for child_id in children {
        apply_css_rule(arena, child_id, selector, css);
    }
}
// css_into_node

pub fn css_into_node(arena: &mut NodeArena, root_id: NodeId, css: &CssRule) {
    for (selector, attr) in css.0.iter() {
        apply_css_rule(arena, root_id, selector, attr);
    }
    // Propagate inheritable CSS properties (color, text-align, font-weight,
    // font-style, text-decoration)
    inherit_css(arena, root_id, &IndexMap::new());
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

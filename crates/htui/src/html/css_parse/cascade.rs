use indexmap::IndexMap;

use crate::tdom::node::{NodeArena, NodeId};

use super::{
    matcher::selector_chain_matches_with_hover, properties::CssAttr, selector::CssSelector,
    stylesheet::{CssRule, CssRuleEntry, parse_declaration_splits},
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
/// Apply the `style` attribute of every node in the subtree, restricted to the
/// `important` subset when `important` is true. Runs as two passes (normal then
/// important) so inline normal declarations beat stylesheet normal ones while
/// inline `!important` beats stylesheet `!important` — the browser cascade
/// order within the author origin.
fn apply_inline_styles(arena: &mut NodeArena, node_id: NodeId, important: bool) {
    let inline_style = arena[node_id].attributes.get("style").cloned();
    if let Some(style) = inline_style {
        let (normal, imp) = parse_declaration_splits(&style);
        let subset = if important { imp } else { normal };
        if !subset.0.is_empty() {
            arena[node_id].css_attr.add(subset);
        }
    }

    let children = arena[node_id].children.clone();
    for child_id in children {
        apply_inline_styles(arena, child_id, important);
    }
}

/// Apply all stylesheet rules, inline `style` attributes and inheritance to
/// the arena, evaluating `:hover` pseudo-class rules against `hovered` (the
/// session-local node currently under the pointer).
///
/// The cascade follows the browser's author-origin order:
///   1. stylesheet normal declarations, ordered by (specificity, source order);
///   2. inline `style` normal declarations;
///   3. stylesheet `!important` declarations, ordered the same way;
///   4. inline `style` `!important` declarations (highest priority).
/// Each later pass overrides earlier ones per property.
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
    // Order every rule by (specificity, source order); the stable sort keeps
    // same-specificity rules in stylesheet order so the last applied rule for
    // a property wins, matching the browser cascade.
    let mut rules: Vec<&CssRuleEntry> = css.0.iter().collect();
    rules.sort_by_key(|entry| entry.selector.specificity());

    // 1. Author stylesheet, normal declarations (lowest priority).
    for entry in &rules {
        if !entry.normal.0.is_empty() {
            apply_css_rule(arena, root_id, &entry.selector, &entry.normal, hovered);
        }
    }
    // 2. Inline `style` attributes, normal declarations.
    apply_inline_styles(arena, root_id, false);
    // 3. Author stylesheet, `!important` declarations.
    for entry in &rules {
        if !entry.important.0.is_empty() {
            apply_css_rule(arena, root_id, &entry.selector, &entry.important, hovered);
        }
    }
    // 4. Inline `style` attributes, `!important` declarations (highest).
    apply_inline_styles(arena, root_id, true);
    // Propagate inheritable CSS properties (color, text-align, font-weight,
    // font-style, text-decoration, font-size, text-transform, line-height).
    // Inheritance copies final values; the importance flag does not propagate.
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
    // Positioning (CSS Positioned Layout)
    "position",
    "z-index",
    "top",
    "right",
    "bottom",
    "left",
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

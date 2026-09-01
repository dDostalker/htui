use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;

use crate::tdom::node::{Node, NodeArena, NodeId, NodeType};

use super::{
    matcher::selector_chain_matches_with_state,
    selector::{DynamicState, PseudoClassSet, Selector},
    stylesheet::{CssRule, parse_declaration_splits},
};

#[derive(Debug, Clone, Default)]
pub(crate) struct CascadeIndex {
    /// Inverse permutation of the cascade order: `rank[rule]` is the rule's
    /// position in `(specificity, source order)` — the browser author-origin
    /// order. Candidates are sorted by it so later-ranked declarations
    /// override earlier ones per property.
    rank: Vec<u32>,
    /// Rules whose rightmost compound contains a tag selector, keyed by tag.
    by_tag: HashMap<NodeType, Vec<u32>>,
    /// Rules whose rightmost compound contains a class selector, keyed by
    /// class token.
    by_class: HashMap<String, Vec<u32>>,
    /// Rules whose rightmost compound contains an id selector, keyed by id.
    by_id: HashMap<String, Vec<u32>>,
    /// Rules whose rightmost compound carries no tag / class / id key (`*`,
    /// `:hover`, `:nth-child(2)`, …) — they must be evaluated for every
    /// element node.
    wildcard: Vec<u32>,
}

impl CascadeIndex {
    /// Build the index for `css`: sort the rules once into cascade order,
    /// then bucket each rule under every key its rightmost compound can
    /// match (a rule with several keys lands in several buckets; the
    /// per-node gather deduplicates).
    pub(crate) fn build(css: &CssRule) -> Self {
        let count = css.0.len();
        // Stable sort keeps same-specificity rules in stylesheet order, so
        // the last applied rule for a property wins, matching the browser
        // cascade.
        let mut order: Vec<u32> = (0..count as u32).collect();
        order.sort_by_key(|&rule| css.0[rule as usize].selector.specificity());
        let mut rank = vec![0u32; count];
        for (position, &rule) in order.iter().enumerate() {
            rank[rule as usize] = position as u32;
        }

        let mut by_tag: HashMap<NodeType, Vec<u32>> = HashMap::new();
        let mut by_class: HashMap<String, Vec<u32>> = HashMap::new();
        let mut by_id: HashMap<String, Vec<u32>> = HashMap::new();
        let mut wildcard: Vec<u32> = Vec::new();
        for rule in 0..count as u32 {
            let Some(compound) = css.0[rule as usize].selector.compounds.last() else {
                // An empty selector chain can never match; keep it out of
                // every bucket so it is never evaluated.
                continue;
            };
            let mut keyed = false;
            for part in compound {
                match part {
                    Selector::Tag(tag) => {
                        by_tag.entry(*tag).or_default().push(rule);
                        keyed = true;
                    }
                    Selector::Class(class) => {
                        by_class.entry(class.clone()).or_default().push(rule);
                        keyed = true;
                    }
                    Selector::Id(id) => {
                        by_id.entry(id.clone()).or_default().push(rule);
                        keyed = true;
                    }
                    // Universal / `:nth-child` / pseudo-class parts restrict
                    // nothing by themselves.
                    _ => {}
                }
            }
            if !keyed {
                wildcard.push(rule);
            }
        }
        CascadeIndex {
            rank,
            by_tag,
            by_class,
            by_id,
            wildcard,
        }
    }

    /// Collect the cascade-ordered candidate rules for `node` into `out`
    /// (cleared first): the wildcard bucket plus every tag / class / id
    /// bucket the node's attributes name, sorted by cascade rank.
    ///
    /// `seen` / `stamp` deduplicate rules that land in several buckets across
    /// one cascade pass: bump `stamp` per node; a rule is taken only when
    /// `seen[rule] != stamp`.
    fn candidates_for(&self, node: &Node, out: &mut Vec<u32>, seen: &mut [u32], stamp: u32) {
        out.clear();
        let empty: Vec<u32> = Vec::new();
        let mut push_bucket = |bucket: &Vec<u32>| {
            for &rule in bucket {
                if seen[rule as usize] != stamp {
                    seen[rule as usize] = stamp;
                    out.push(rule);
                }
            }
        };
        push_bucket(&self.wildcard);
        push_bucket(self.by_tag.get(&node.node_type).unwrap_or(&empty));
        if let Some(classes) = node.attributes.get("class") {
            for class in classes.split_whitespace() {
                push_bucket(self.by_class.get(class).unwrap_or(&empty));
            }
        }
        if let Some(id) = node.get_id() {
            push_bucket(self.by_id.get(id).unwrap_or(&empty));
        }
        out.sort_unstable_by_key(|&rule| self.rank[rule as usize]);
    }
}

/// Apply the `style` attribute of one node, restricted to the `important`
/// subset when `important` is true the per-node form of the cascade's
/// inline passes (inline normal beats stylesheet normal, inline `!important`
/// beats everything).
fn apply_inline_style_at(arena: &mut NodeArena, node_id: NodeId, important: bool) {
    let Some(style) = arena[node_id].attributes.get("style").cloned() else {
        return;
    };
    let (normal, imp) = parse_declaration_splits(&style);
    let subset = if important { imp } else { normal };
    if !subset.0.is_empty() {
        arena[node_id].css_attr.add(&subset);
    }
}

/// Apply all stylesheet rules, inline `style` attributes and inheritance to
/// the arena, evaluating the dynamic pseudo-classes (`:hover` / `:focus`)
/// against the session [`DynamicState`] snapshot.
///
/// The cascade follows the browser's author-origin order:
///   1. stylesheet normal declarations, ordered by (specificity, source order);
///   2. inline `style` normal declarations;
///   3. stylesheet `!important` declarations, ordered the same way;
///   4. inline `style` `!important` declarations (highest priority).
///
/// Each later pass overrides earlier ones per property.
///
/// Callers that need a state-aware cascade (the event system's hover / focus
/// highlight) use this; everything else should call [`css_into_node`], which
/// cascades without any dynamic pseudo-class matching.
pub fn css_into_node_with_state(
    arena: &mut NodeArena,
    root_id: NodeId,
    css: &CssRule,
    state: &DynamicState,
) {
    let index = css.cascade_index();
    // Per-node evaluation scratch, reused across the whole pass so neither
    // the traversal nor the candidate gathering allocates per node / rule.
    let mut seen = vec![0u32; css.0.len()];
    let mut candidates: Vec<u32> = Vec::new();
    let mut stack = vec![root_id];
    let mut stamp: u32 = 0;
    while let Some(node_id) = stack.pop() {
        stamp = stamp.wrapping_add(1);
        if stamp == 0 {
            // The generation stamp wrapped around: re-zero the stamps and
            // resume at 1 so stale marks can never suppress a candidate.
            seen.fill(0);
            stamp = 1;
        }
        index.candidates_for(&arena[node_id], &mut candidates, &mut seen, stamp);
        // 1. Author stylesheet, normal declarations (lowest priority).
        for &rule in &candidates {
            let entry = &css.0[rule as usize];
            if entry.normal.0.is_empty() {
                continue;
            }
            if selector_chain_matches_with_state(arena, node_id, &entry.selector, state) {
                arena[node_id].css_attr.add(&entry.normal);
            }
        }
        // 2. Inline `style` attributes, normal declarations.
        apply_inline_style_at(arena, node_id, false);
        // 3. Author stylesheet, `!important` declarations.
        for &rule in &candidates {
            let entry = &css.0[rule as usize];
            if entry.important.0.is_empty() {
                continue;
            }
            if selector_chain_matches_with_state(arena, node_id, &entry.selector, state) {
                arena[node_id].css_attr.add(&entry.important);
            }
        }
        // 4. Inline `style` attributes, `!important` declarations (highest).
        apply_inline_style_at(arena, node_id, true);

        stack.extend(arena[node_id].children.iter().copied());
    }
    // Propagate inheritable CSS properties (color, text-align, font-weight,
    // font-style, text-decoration, font-size, text-transform, line-height).
    // Inheritance copies final values; the importance flag does not propagate.
    inherit_css(arena, root_id);
}

/// Cascade styles with a hover context but no focus (no `:focus` rules match).
pub fn css_into_node_with_hover(
    arena: &mut NodeArena,
    root_id: NodeId,
    css: &CssRule,
    hovered: Option<NodeId>,
) {
    css_into_node_with_state(
        arena,
        root_id,
        css,
        &DynamicState {
            hovered,
            focused: None,
        },
    );
}

/// Cascade styles with no dynamic pseudo-class context (no `:hover` / `:focus`
/// rule matches).
pub fn css_into_node(arena: &mut NodeArena, root_id: NodeId, css: &CssRule) {
    css_into_node_with_state(arena, root_id, css, &DynamicState::default());
}

/// The set of dynamic pseudo-classes any rule in `css` uses — computed once at
/// rule-build time so state-change commands can test membership in O(1)
/// instead of rescanning the stylesheet.
pub fn css_pseudo_rule_set(css: &CssRule) -> PseudoClassSet {
    css.0.iter().fold(PseudoClassSet::EMPTY, |set, entry| {
        set.union(PseudoClassSet::from_selector(&entry.selector))
    })
}

/// Clear the parsed CSS attributes of every node in the subtree rooted at
/// `root_id`, preparing the arena for a fresh [`css_into_node_with_hover`]
/// pass (the cascade only ever adds, so re-running it requires a reset).
pub fn reset_css_attr(arena: &mut NodeArena, root_id: NodeId) {
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        // Clear in place to reuse the map's capacity across passes instead of
        // allocating a fresh `IndexMap` per node per frame.
        arena[id].css_attr.0.clear();
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
    "caret-color",
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
    "border-top-width",
    "border-top-style",
    "border-top-color",
    "border-right-width",
    "border-right-style",
    "border-right-color",
    "border-bottom-width",
    "border-bottom-style",
    "border-bottom-color",
    "border-left-width",
    "border-left-style",
    "border-left-color",
    "border-radius",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-right-radius",
    "border-bottom-left-radius",
    // Flexbox
    "display",
    "flex",
    "flex-flow",
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
    "place-content",
    "place-items",
    "place-self",
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
    // Multi-column layout (CSS Multi-column Layout Level 1)
    "columns",
    "column-width",
    "column-count",
    "column-gap",
    "column-fill",
    "column-rule",
    "column-rule-width",
    "column-rule-style",
    "column-rule-color",
    // Visibility / overflow
    "visibility",
    "overflow",
    "overflow-x",
    "overflow-y",
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
    "visibility",
];

fn inherit_css(arena: &mut NodeArena, root_id: NodeId) {
    type InheritedMap = IndexMap<Arc<str>, Arc<str>>;
    // Iterative pre-order walk with an explicit stack. An implicit-recursive
    // version would clone each node's `children` `Vec` on the way down
    // (one allocation per node); the shared stack copies each child `NodeId`
    // exactly once and keeps the inherited-view maps behind `Arc` so whole
    // subtrees that don't diverge share one allocation.
    let mut stack: Vec<(NodeId, Arc<InheritedMap>)> =
        vec![(root_id, Arc::new(InheritedMap::new()))];
    while let Some((node_id, parent_attrs)) = stack.pop() {
        // Merge parent inheritable attrs into current node (if not already
        // set). `entry` does a single hash probe for the find-or-insert; the
        // eager key allocation is cheaper than a second `contains_key` probe
        // on this hot per-node path.
        {
            let node = &mut arena[node_id];
            for key in INHERITED_PROPS {
                if let Some(value) = parent_attrs.get(*key) {
                    node.css_attr
                        .0
                        .entry(Arc::from(*key))
                        .or_insert_with(|| value.clone());
                }
            }
        }

        // The inherited map handed down to children: share the parent's unless
        // this node declares an inherited property that differs from it
        // (copy-on-write — most nodes change nothing, so whole subtrees share
        // one map allocation instead of cloning it per node).
        let merged = {
            let node = &arena[node_id];
            let diverges = INHERITED_PROPS.iter().any(|key| {
                node.css_attr
                    .0
                    .get(*key)
                    .is_some_and(|value| parent_attrs.get(*key) != Some(value))
            });
            if diverges {
                let mut map = (*parent_attrs).clone();
                for key in INHERITED_PROPS {
                    if let Some(value) = node.css_attr.0.get(*key) {
                        map.insert(Arc::from(*key), value.clone());
                    }
                }
                Arc::new(map)
            } else {
                Arc::clone(&parent_attrs)
            }
        };

        for child in arena[node_id].children.iter().rev() {
            stack.push((*child, Arc::clone(&merged)));
        }
    }
}

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;

use crate::tdom::node::{Node, NodeArena, NodeId, NodeType};

use super::{
    matcher::selector_chain_matches_with_state,
    selector::{DynamicState, PseudoClassSet, Selector},
    stylesheet::{CssRule, parse_declaration_splits},
    values::Viewport,
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

/// Insert every declaration under its pseudo-element-prefixed key
/// (`color` → `color::before`), the slot namespace generated content reads.
fn add_prefixed(
    target: &mut crate::html::css_parse::properties::CssAttr,
    css: &crate::html::css_parse::properties::CssAttr,
    pe: crate::html::css_parse::PseudoElement,
) {
    let suffix = match pe {
        crate::html::css_parse::PseudoElement::Before => "::before",
        crate::html::css_parse::PseudoElement::After => "::after",
        crate::html::css_parse::PseudoElement::SliderThumb => "::slider-thumb",
        crate::html::css_parse::PseudoElement::SliderTrack => "::slider-track",
        crate::html::css_parse::PseudoElement::SliderProgress => "::slider-progress",
    };
    for (k, v) in &css.0 {
        let key: Arc<str> = Arc::from(format!("{k}{suffix}"));
        target.0.insert(key, Arc::clone(v));
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

// CSS custom property (variable) substitution:
/// Substitute `var(--name)` / `var(--name, fallback)` references in a CSS
/// property value using the current custom-property scope.
///
/// Called after the cascade resolves all declarations for a node, so the
/// substitution sees the final cascaded value (matching the browser's
/// computed-value timing). Recursion handles variables that reference other
/// variables; a depth cap prevents runaway cycles.
fn substitute_vars(input: &str, vars: &HashMap<String, String>, depth: usize) -> String {
    const MAX_DEPTH: usize = 16;
    if depth > MAX_DEPTH || !input.contains("var(") {
        return input.to_string();
    }
    let mut result = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find("var(") {
        result.push_str(&rest[..pos]);
        let after = &rest[pos + 4..]; // past `var(`
        let Some(close) = find_matching_paren(after) else {
            // Malformed — keep the text as-is and continue scanning.
            result.push_str("var(");
            rest = after;
            continue;
        };
        let inner = &after[..close];
        rest = &after[close + 1..];

        let (name, fallback) = split_var_args(inner);
        let name = name.trim();
        if let Some(value) = vars.get(name) {
            let resolved = substitute_vars(value, vars, depth + 1);
            result.push_str(&resolved);
        } else if let Some(fb) = fallback {
            let resolved = substitute_vars(fb, vars, depth + 1);
            let trimmed = fb_style(&resolved);
            result.push_str(&trimmed);
        } else {
            // Unresolved variable with no fallback: the property becomes
            // "invalid at computed-value time" in browsers (treated as
            // `unset`). We substitute with an empty string so the
            // downstream parser silently ignores it.
        }
    }
    result.push_str(rest);
    result
}

/// Fallback values keep their surrounding whitespace trimmed.
fn fb_style(s: &str) -> String {
    s.trim().to_string()
}

/// Find the index of the `)` that closes the paren opened at position 0 of
/// `s` (which starts right after `var(`). Handles nesting.
fn find_matching_paren(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 1usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split `var(...)` inner content into `(name, Option<fallback>)` at the
/// first top-level comma (commas inside nested parens are kept).
fn split_var_args(inner: &str) -> (&str, Option<&str>) {
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                return (&inner[..i], Some(inner[i + 1..].trim()));
            }
            _ => {}
        }
    }
    (inner, None)
}

/// Extract all custom properties (`--x: value`) from a node's cascaded
/// attributes and merge them into `parent_vars`. Returns the merged scope
/// with each value recursively resolved against the merged map.
fn resolve_custom_props(
    css_attr: &crate::html::css_parse::properties::CssAttr,
    parent_vars: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut vars = parent_vars;
    for (k, v) in &css_attr.0 {
        if k.starts_with("--") {
            vars.insert(k.to_string(), v.to_string());
        }
    }
    // Resolve `var()` inside custom property values (e.g. `--a: var(--b)`).
    let entries: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (k, raw) in entries {
        if raw.contains("var(") {
            let resolved = substitute_vars(&raw, &vars, 0);
            vars.insert(k, resolved);
        }
    }
    vars
}

/// Replace `var(--name)` references in every non-custom (`--`-prefixed)
/// property value of the node's cascaded attributes.
fn substitute_property_vars(
    css_attr: &mut crate::html::css_parse::properties::CssAttr,
    vars: &HashMap<String, String>,
) {
    let keys: Vec<Arc<str>> = css_attr
        .0
        .keys()
        .filter(|k| !k.starts_with("--"))
        .cloned()
        .collect();
    for key in keys {
        if let Some(v) = css_attr.0.get_mut(&key)
            && v.contains("var(")
        {
            let resolved = substitute_vars(v, vars, 0);
            *v = Arc::from(resolved.as_str());
        }
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
    // `@media` visibility is fixed for the whole pass (the viewport is part
    // of the state), so it is decided once per rule instead of per node.
    let media_active: Vec<bool> = css
        .0
        .iter()
        .map(|e| e.media.as_ref().is_none_or(|m| m.matches(&state.viewport)))
        .collect();
    // Per-node evaluation scratch, reused across the whole pass so neither
    // the traversal nor the candidate gathering allocates per node / rule.
    let mut seen = vec![0u32; css.0.len()];
    let mut candidates: Vec<u32> = Vec::new();
    let mut stack = vec![root_id];
    let mut stamp: u32 = 0;
    // Per-node custom-property scope, keyed by child NodeId and populated
    // just before pushing children onto the stack.
    let mut var_scopes: HashMap<NodeId, HashMap<String, String>> = HashMap::new();
    while let Some(node_id) = stack.pop() {
        stamp = stamp.wrapping_add(1);
        if stamp == 0 {
            // The generation stamp wrapped around: re-zero the stamps and
            // resume at 1 so stale marks can never suppress a candidate.
            seen.fill(0);
            stamp = 1;
        }
        index.candidates_for(&arena[node_id], &mut candidates, &mut seen, stamp);
        // Drop rules whose `@media` condition does not match the viewport
        // (order-preserving, so the cascade rank survives the filter).
        candidates.retain(|&rule| media_active[rule as usize]);
        // 1. Author stylesheet, normal declarations (lowest priority).
        for &rule in &candidates {
            let entry = &css.0[rule as usize];
            if entry.normal.0.is_empty() {
                continue;
            }
            if selector_chain_matches_with_state(arena, node_id, &entry.selector, state) {
                if let Some(pe) = entry.selector.pseudo_element {
                    // Pseudo-element declarations live under key-prefixed
                    // slots (`content::before`, `color::before`, …) so they
                    // cascade among themselves without touching the origin
                    // element's own properties.
                    add_prefixed(&mut arena[node_id].css_attr, &entry.normal, pe);
                } else {
                    arena[node_id].css_attr.add(&entry.normal);
                }
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
                if let Some(pe) = entry.selector.pseudo_element {
                    add_prefixed(&mut arena[node_id].css_attr, &entry.important, pe);
                } else {
                    arena[node_id].css_attr.add(&entry.important);
                }
            }
        }
        // 4. Inline `style` attributes, `!important` declarations (highest).
        apply_inline_style_at(arena, node_id, true);

        // 5. CSS custom properties (variables): merge the node's own
        // `--name: value` declarations with the parent scope, then
        // substitute `var(--name)` references in every non-custom property
        // value (matching the browser's computed-value timing).
        let parent_vars = var_scopes.remove(&node_id).unwrap_or_default();
        let node_vars = resolve_custom_props(&arena[node_id].css_attr, parent_vars);
        substitute_property_vars(&mut arena[node_id].css_attr, &node_vars);

        for &child in &arena[node_id].children {
            var_scopes.insert(child, node_vars.clone());
            stack.push(child);
        }
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
            viewport: Viewport { cols: 0, rows: 0 },
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
///   2. Add an accessor on [`CssAttr`](crate::html::css_parse::CssAttr) (e.g. `get_padding()`).
///   3. If it should inherit, also append to [`INHERITED_PROPS`].
///
/// No other site needs editing — the cascade in `inherit_css` drives itself
/// from these lists.
pub const SUPPORTED_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "background",
    "background-clip",
    "caret-color",
    "accent-color",
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
    // Text spacing / clipping
    "letter-spacing",
    "word-spacing",
    "text-indent",
    "text-overflow",
    "user-select",
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
    // Lists
    "list-style",
    "list-style-type",
    // Tables
    "table-layout",
    "caption-side",
    "empty-cells",
    // Font shorthand (expanded at parse time into the font-* longhands)
    "font",
];

/// Inheritable subset of [`SUPPORTED_PROPERTIES`]: a child without an explicit
/// value picks up its parent's.
///
/// Kept as a separate list because Rust const-fn filtering isn't yet stable;
/// appending a name here is what flips a property to inheritable.
///
/// This mirrors the CSS-inherited set for the properties we support, with one
/// deliberate deviation: `text-decoration` is **not inherited** per CSS spec
/// (it *propagates* visually down an inline box), but the terminal renderer
/// collapses an inline chain into merged runs, so treating it as inherited
/// reproduces the browser's visual propagation (an underlined `<a>` keeps its
/// underline through nested inline elements). The whitespace / wrapping trio
/// (`white-space`, `word-break`, `overflow-wrap`) is inherited exactly as in
/// CSS Text, so `div { white-space: pre }` propagates to inline children.
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
    "white-space",
    "word-break",
    "overflow-wrap",
    "list-style-type",
    "empty-cells",
    "letter-spacing",
    "word-spacing",
    "text-indent",
    "accent-color",
];

fn inherit_css(arena: &mut NodeArena, root_id: NodeId) {
    /// The value `initial` (and `inherit` without a parent value) resolves
    /// to: unparseable by every getter, so reads fall back to the built-in
    /// default, while occupying the cascade slot so the inherited merge
    /// cannot refill it.
    const INITIAL_SENTINEL: &str = "\u{1}initial";
    type InheritedMap = IndexMap<Arc<str>, Arc<str>>;
    // Iterative pre-order walk with an explicit stack. An implicit-recursive
    // version would clone each node's `children` `Vec` on the way down
    // (one allocation per node); the shared stack copies each child `NodeId`
    // exactly once and keeps the inherited-view maps behind `Arc` so whole
    // subtrees that don't diverge share one allocation.
    let mut stack: Vec<(NodeId, Arc<InheritedMap>)> =
        vec![(root_id, Arc::new(InheritedMap::new()))];
    while let Some((node_id, parent_attrs)) = stack.pop() {
        // Explicit global keywords, resolved before the inherited merge so
        // the literal value cannot squat on the slot:
        // - `inherit` takes the parent's **computed** value (any property —
        //   the non-inherited set reads the parent node's css_attr directly);
        //   with no parent value it falls to the initial sentinel below.
        // - `initial` swaps the declaration for the [`INITIAL_SENTINEL`]
        //   value: getters fail to parse it and fall back to their built-in
        //   default (the engine's initial-value analogue), and the sentinel
        //   occupies the slot so the inherited merge cannot refill it —
        //   which is exactly the keyword's point on inherited properties.
        // `unset` / `revert` / `revert-layer` are not recognized (no cascade
        // origin table to revert to).
        {
            let parent_id = arena.get(node_id).and_then(|n| n.parent);
            // Collect (key, parent-value-if-inherit) under shared borrows.
            let mut resolves: Vec<(Arc<str>, Option<Arc<str>>)> = Vec::new();
            {
                let node = &arena[node_id];
                for (key, value) in node.css_attr.0.iter() {
                    let text = value.trim();
                    if text.eq_ignore_ascii_case("inherit") {
                        let parent_value = parent_id
                            .and_then(|pid| arena.get(pid))
                            .and_then(|p| p.css_attr.0.get(key).cloned());
                        resolves.push((key.clone(), parent_value));
                    } else if text.eq_ignore_ascii_case("initial") {
                        resolves.push((key.clone(), None));
                    }
                }
            }
            if !resolves.is_empty() {
                let node = &mut arena[node_id];
                for (key, value) in resolves {
                    match value {
                        Some(v) => {
                            node.css_attr.0.insert(key, v);
                        }
                        None => {
                            node.css_attr.0.insert(key, Arc::from(INITIAL_SENTINEL));
                        }
                    }
                }
            }
        }

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

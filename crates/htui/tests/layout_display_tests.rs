//! Tests for the unified `display` model.
//!
//! Verifies that the generation phase honours every `display` keyword
//! the renderer cares about:
//!
//! - `display: none` removes the box (and, at the layout level, its subtree)
//! - `display: contents` classifies as a transparent marker whose children
//!   splice into the parent's sibling list
//! - `display: inline-flex` / `inline-block` route to the same algorithms
//!   as `flex` / `block` (inline-flex produces `FormattingContext::Flex`)
//! - `display: table` on a non-`<table>` element still classifies as
//!   `FormattingContext::Table`
//! - `<img>` without an explicit `display` keeps its UA-default
//!   `inline-block` outer role and `Replaced` box kind
//! - end-to-end: a hidden subtree renders nothing
//!
//! Generation is a *shallow per-node classifier* (the geometry phase
//! re-walks the DOM), so these tests assert through `generate_node` on
//! individual nodes and through the rendered canvas — not by walking a
//! generated tree.

use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{
    FormattingContext, GeneratedBox, GeneratedBoxKind, GeneratedPayload, OuterDisplay,
    flatten_contents_children, generate_node,
};
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::TermLayout;

fn setup(html: &str, css: &str) -> (NodeId, NodeArena) {
    let mut p = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = p.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    // Always call `css_into_node` (even when `combined` is empty) so that
    // `style="..."` attributes on individual nodes get applied via
    // `apply_inline_styles`. An empty CssRule is a no-op for the cascade
    // pass itself, but the inline-styles sweep still runs.
    let mut cp = css_parse::Parser::new(&combined);
    css_parse::css_into_node(&mut arena, root_id, &cp.parse());
    (root_id, arena)
}

/// The DOM id of the first node matching `pred`, in document order.
fn find_node_id(
    arena: &NodeArena,
    root_id: NodeId,
    pred: impl Fn(&NodeType) -> bool + Copy,
) -> NodeId {
    fn walk<F: Fn(&NodeType) -> bool + Copy>(
        arena: &NodeArena,
        id: NodeId,
        pred: &F,
    ) -> Option<NodeId> {
        let node = &arena[id];
        if pred(&node.node_type) {
            return Some(id);
        }
        node.children.iter().find_map(|&c| walk(arena, c, pred))
    }
    walk(arena, root_id, &pred).expect("node present in document")
}

/// Classify one DOM node through the shallow generation entry point the
/// layout walks use.
fn classify(arena: &NodeArena, id: NodeId) -> GeneratedBox {
    generate_node(arena, id).unwrap_or_else(|| panic!("node should generate a box"))
}

fn layout_canvas(html: &str, css: &str) -> String {
    let (root_id, arena) = setup(html, css);
    let term_map = TermMap {
        columns: 80,
        rows: 24,
    };
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    tl.layout.iter().map(|b| b.word.as_str()).collect()
}

#[test]
fn display_none_excludes_box_from_tree() {
    let (root_id, arena) = setup(
        "<div><p>visible</p><p style=\"display: none\">gone</p></div>",
        "",
    );
    let hidden_p = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::P));
    // Both <p>s share the node type — pick them apart by text content.
    let all_ps: Vec<NodeId> = {
        let mut out = Vec::new();
        let mut stack = vec![root_id];
        while let Some(id) = stack.pop() {
            if matches!(arena[id].node_type, NodeType::P) {
                out.push(id);
            }
            stack.extend(arena[id].children.iter().copied());
        }
        out
    };
    let (hidden, visible) = all_ps
        .into_iter()
        .partition::<Vec<_>, _>(|&id| arena[id].text_content(&arena) == "gone");
    assert_eq!(hidden.len(), 1);
    assert_eq!(visible.len(), 1);
    assert!(
        generate_node(&arena, hidden[0]).is_none(),
        "display: none <p> must generate no box"
    );
    assert!(
        generate_node(&arena, visible[0]).is_some(),
        "the visible <p> should generate a box"
    );
    let _ = hidden_p; // (node-type search is ambiguous; text partition above is exact)
}

#[test]
fn display_none_subtree_also_excluded() {
    let (root_id, arena) = setup(
        "<div style=\"display: none\"><p>never</p><span>shown</span></div>",
        "",
    );
    // The div is a top-level child: the shallow top-level forest must skip
    // it entirely (its descendants are never even visited).
    let boxes = htui::layout::generate_box_tree(&arena, root_id);
    assert!(
        boxes
            .iter()
            .all(|g| !matches!(arena[g.node_id].node_type, NodeType::Div)),
        "display: none div and all its descendants must be absent"
    );
    // And the rendered page shows nothing.
    assert!(
        !layout_canvas(
            "<div style=\"display: none\"><p>never</p><span>shown</span></div>",
            ""
        )
        .contains("never"),
        "a display: none subtree must not render"
    );
}

#[test]
fn hidden_attribute_excludes_box_from_tree() {
    // The UA rule `[hidden] { display: none }`: an element carrying the
    // `hidden` global attribute produces no box (matching the browser, where
    // `hidden` behaves like `display: none` unless an author `display` wins).
    let (root_id, arena) = setup("<div><p>visible</p><p hidden>gone</p></div>", "");
    let ps: Vec<NodeId> = {
        let mut out = Vec::new();
        let mut stack = vec![root_id];
        while let Some(id) = stack.pop() {
            if matches!(arena[id].node_type, NodeType::P) {
                out.push(id);
            }
            stack.extend(arena[id].children.iter().copied());
        }
        out
    };
    let (hidden, visible) = ps
        .into_iter()
        .partition::<Vec<_>, _>(|&id| arena[id].attributes.contains_key("hidden"));
    assert_eq!(hidden.len(), 1);
    assert_eq!(visible.len(), 1);
    assert!(
        generate_node(&arena, hidden[0]).is_none(),
        "the hidden <p> must generate no box"
    );
    assert!(generate_node(&arena, visible[0]).is_some());
    // End to end: the hidden element's content never renders.
    assert!(
        !layout_canvas("<div><p>visible</p><p hidden>gone</p></div>", "").contains("gone"),
        "the hidden <p>'s content must not render"
    );
}

#[test]
fn hidden_attribute_drops_the_subtree() {
    let (root_id, arena) = setup("<div hidden><p>never</p><span>shown</span></div>", "");
    let boxes = htui::layout::generate_box_tree(&arena, root_id);
    assert!(
        boxes
            .iter()
            .all(|g| !matches!(arena[g.node_id].node_type, NodeType::Div)),
        "a hidden div and all its descendants must be absent"
    );
}

#[test]
fn hidden_attribute_yields_to_author_display() {
    // `hidden` is a plain UA rule, so an author `display` overrides it
    // (browser behaviour): `style="display: block"` on a `hidden` element
    // still renders.
    let (root_id, arena) = setup(
        "<div><p hidden style=\"display: block\">shown</p></div>",
        "",
    );
    let p = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::P));
    assert!(
        generate_node(&arena, p).is_some(),
        "an author display on a hidden element must keep it rendering"
    );
}

#[test]
fn display_contents_classifies_as_transparent_marker() {
    let (root_id, arena) = setup(
        "<div><span style=\"display: contents\">X</span><p>after</p></div>",
        "",
    );
    let span = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::Span));
    let box_ = classify(&arena, span);
    assert_eq!(box_.kind, GeneratedBoxKind::Anonymous);
    assert_eq!(box_.payload, GeneratedPayload::Transparent);
    // The splice helper replaces the marker with its children in the
    // sibling list — this is what the layout walks consume.
    let div = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::Div));
    let spliced = flatten_contents_children(&arena, &arena[div].children);
    let p = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::P));
    assert_eq!(
        spliced,
        vec![span.children_(&arena)[0], p],
        "the contents span's children take its place in document order"
    );
    // End to end: the flattened text renders inside the parent's flow.
    assert!(
        layout_canvas(
            "<div><span style=\"display: contents\">X</span><p>after</p></div>",
            ""
        )
        .contains("X"),
        "the text 'X' must survive flattening"
    );
}

#[test]
fn display_inline_flex_is_flex_formatting_context() {
    let (root_id, arena) = setup("<div style=\"display: inline-flex\">x</div>", "");
    let div = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::Div));
    let box_ = classify(&arena, div);
    assert_eq!(box_.inner_formatting_context, FormattingContext::Flex);
    assert!(
        matches!(box_.outer_display, OuterDisplay::Inline { block: true }),
        "inline-flex should yield Inline {{ block: true }}, got {:?}",
        box_.outer_display
    );
}

#[test]
fn display_inline_block_is_inline_outer_with_block_fc() {
    let (root_id, arena) = setup("<div style=\"display: inline-block\">x</div>", "");
    let div = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::Div));
    let box_ = classify(&arena, div);
    assert!(matches!(
        box_.outer_display,
        OuterDisplay::Inline { block: true }
    ));
    assert_eq!(box_.inner_formatting_context, FormattingContext::Flow);
}

#[test]
fn display_table_on_non_table_element_uses_table_fc() {
    let (root_id, arena) = setup(
        "<div style=\"display: table\">\
            <div style=\"display: table-row\">\
                <div style=\"display: table-cell\">c</div>\
            </div>\
        </div>",
        "",
    );
    let div = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::Div));
    let box_ = classify(&arena, div);
    assert_eq!(box_.inner_formatting_context, FormattingContext::Table);
    assert!(matches!(
        box_.outer_display,
        OuterDisplay::BlockLevel { container: true }
    ));
}

#[test]
fn img_uses_ua_default_inline_block_outer_display() {
    let (root_id, arena) = setup("<img src='x.png'>", "");
    let img = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::Img));
    let box_ = classify(&arena, img);
    assert_eq!(box_.kind, GeneratedBoxKind::Replaced);
    assert!(
        matches!(box_.outer_display, OuterDisplay::Inline { block: true }),
        "<img> UA default should be inline-block, got {:?}",
        box_.outer_display
    );
}

#[test]
fn img_explicit_block_outer_overrides_ua_default() {
    let (root_id, arena) = setup("<img src='x.png' style='display: block'>", "");
    let img = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::Img));
    let box_ = classify(&arena, img);
    assert_eq!(box_.kind, GeneratedBoxKind::Replaced);
    assert!(
        matches!(
            box_.outer_display,
            OuterDisplay::BlockLevel { container: true }
        ),
        "explicit display: block must win, got {:?}",
        box_.outer_display
    );
}

#[test]
fn display_grid_is_grid_formatting_context() {
    let (root_id, arena) = setup("<div style=\"display: grid\">x</div>", "");
    let div = find_node_id(&arena, root_id, |nt| matches!(nt, NodeType::Div));
    let box_ = classify(&arena, div);
    assert_eq!(box_.inner_formatting_context, FormattingContext::Grid);
}

// --- end-to-end geometry tests -------------------------------------------
//
// The classification tests above pin the per-node semantics; these pin the
// *layout* behaviour: a `display: contents` element produces no box, so its
// children must be spliced into the parent's layout walk (flow and flex)
// instead of being silently dropped.

#[test]
fn display_contents_children_render_in_flow() {
    let canvas = layout_canvas(
        "<div>before<div style=\"display: contents\"><p>inner</p></div>after</div>",
        "",
    );
    assert!(
        canvas.contains("inner"),
        "the contents element's children must render in the parent's flow"
    );
    assert!(
        canvas.contains("after"),
        "siblings after the contents element must still render"
    );
}

#[test]
fn display_contents_flattens_recursively() {
    let canvas = layout_canvas(
        "<div style=\"display: contents\"><div style=\"display: contents\"><p>deep</p></div></div>",
        "",
    );
    assert!(
        canvas.contains("deep"),
        "nested display: contents elements must flatten recursively"
    );
}

#[test]
fn display_contents_children_become_flex_items() {
    let canvas = layout_canvas(
        "<div style=\"display: flex\"><div style=\"display: contents\"><p>alpha</p><p>beta</p></div></div>",
        "",
    );
    assert!(
        canvas.contains("alpha") && canvas.contains("beta"),
        "the contents element's children must become flex items, not vanish"
    );
}

// --- grid fallback consistency -------------------------------------------
//
// The grid layout algorithm is a documented reject: `display: grid` /
// `inline-grid` produce no box. The fallback must be the same for both
// variants — the box is skipped and the element contributes no content at
// all. An `inline-grid` element used to leak its text into the parent's
// inline runs as plain text while a block-level grid sibling's content
// vanished silently.

#[test]
fn grid_fallback_is_uniform_between_block_and_inline_variants() {
    let canvas = layout_canvas(
        "<div>a<div style=\"display: grid\">G</div>b<span style=\"display: inline-grid\">H</span>c</div>",
        "",
    );
    assert!(
        canvas.contains("a") && canvas.contains("b") && canvas.contains("c"),
        "the surrounding text must render: {canvas:?}"
    );
    assert!(
        !canvas.contains('G') && !canvas.contains('H'),
        "grid content must not render in either variant — and the inline-grid \
         text must not leak into the parent's inline runs: {canvas:?}"
    );
}

/// `Vec<NodeId>` helper for reading a node's children in tests.
trait ChildrenExt {
    fn children_(&self, arena: &NodeArena) -> Vec<NodeId>;
}

impl ChildrenExt for NodeId {
    fn children_(&self, arena: &NodeArena) -> Vec<NodeId> {
        arena[*self].children.clone()
    }
}

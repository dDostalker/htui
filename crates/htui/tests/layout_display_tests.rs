//! Tests for the unified `display` model.
//!
//! Verifies that the generation phase honours every `display` keyword
//! the renderer cares about:
//!
//! - `display: none` removes the box and its subtree
//! - `display: contents` flattens the element's children into the parent
//! - `display: inline-flex` / `inline-block` route to the same algorithms
//!   as `flex` / `block` (inline-flex produces `FormattingContext::Flex`)
//! - `display: table` on a non-`<table>` element still classifies as
//!   `FormattingContext::Table`
//! - `<img>` without an explicit `display` keeps its UA-default
//!   `inline-block` outer role and `Replaced` box kind
//! - `display: none` on a parent also removes its descendants

use htui::html::{css_parse, html_parse};
use htui::layout::{
    FormattingContext, GeneratedBox, GeneratedBoxKind, OuterDisplay, generate_box_tree,
};
use htui::tdom::node::{NodeArena, NodeId, NodeType};

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

fn find<'a>(
    boxes: &'a [GeneratedBox],
    arena: &NodeArena,
    pred: impl Fn(&NodeType) -> bool + Copy,
) -> Option<&'a GeneratedBox> {
    boxes.iter().find_map(|g| {
        if pred(&arena[g.node_id].node_type) {
            Some(g)
        } else {
            find(&g.children, arena, pred)
        }
    })
}

fn flatten_text_contains(boxes: &[GeneratedBox], arena: &NodeArena, needle: &str) -> bool {
    fn walk(boxes: &[GeneratedBox], arena: &NodeArena, needle: &str, found: &mut bool) {
        for g in boxes {
            if arena[g.node_id].node_type == NodeType::Text
                && arena[g.node_id].data.as_deref() == Some(needle)
            {
                *found = true;
                return;
            }
            walk(&g.children, arena, needle, found);
            if *found {
                return;
            }
        }
    }
    let mut found = false;
    walk(boxes, arena, needle, &mut found);
    found
}

#[test]
fn display_none_excludes_box_from_tree() {
    let (root_id, arena) = setup(
        "<div><p>visible</p><p style=\"display: none\">gone</p></div>",
        "",
    );
    let boxes = generate_box_tree(&arena, root_id);
    let div = find(&boxes, &arena, |nt| matches!(nt, NodeType::Div))
        .expect("div should generate a box");
    let ps: Vec<&GeneratedBox> = div
        .children
        .iter()
        .filter(|c| matches!(arena[c.node_id].node_type, NodeType::P))
        .collect();
    assert_eq!(ps.len(), 1, "display: none <p> must be absent");
    assert!(
        flatten_text_contains(&div.children, &arena, "visible"),
        "the visible <p> should remain in the tree"
    );
}

#[test]
fn display_none_subtree_also_excluded() {
    let (root_id, arena) = setup(
        "<div style=\"display: none\"><p>never</p><span>shown</span></div>",
        "",
    );
    let boxes = generate_box_tree(&arena, root_id);
    let has_div = boxes
        .iter()
        .any(|g| matches!(arena[g.node_id].node_type, NodeType::Div));
    assert!(!has_div, "display: none div and all its descendants must be absent");
}

#[test]
fn display_contents_flattens_children_into_parent() {
    let (root_id, arena) = setup(
        "<div><span style=\"display: contents\">X</span><p>after</p></div>",
        "",
    );
    let boxes = generate_box_tree(&arena, root_id);
    let div = find(&boxes, &arena, |nt| matches!(nt, NodeType::Div))
        .expect("div should generate a box");
    // The contents <span> itself must not appear as a box.
    assert!(
        div.children
            .iter()
            .all(|c| !matches!(arena[c.node_id].node_type, NodeType::Span)),
        "the contents <span> must be absent"
    );
    // The flattened text "X" must still be reachable somewhere in the tree.
    assert!(
        flatten_text_contains(&div.children, &arena, "X"),
        "the text 'X' must survive flattening"
    );
}

#[test]
fn display_inline_flex_is_flex_formatting_context() {
    let (root_id, arena) = setup(
        "<div style=\"display: inline-flex\">x</div>",
        "",
    );
    let boxes = generate_box_tree(&arena, root_id);
    let div = find(&boxes, &arena, |nt| matches!(nt, NodeType::Div))
        .expect("div should generate a box");
    assert_eq!(div.inner_formatting_context, FormattingContext::Flex);
    assert!(
        matches!(div.outer_display, OuterDisplay::Inline { block: true }),
        "inline-flex should yield Inline {{ block: true }}, got {:?}",
        div.outer_display
    );
}

#[test]
fn display_inline_block_is_inline_outer_with_block_fc() {
    let (root_id, arena) = setup(
        "<div style=\"display: inline-block\">x</div>",
        "",
    );
    let boxes = generate_box_tree(&arena, root_id);
    let div = find(&boxes, &arena, |nt| matches!(nt, NodeType::Div))
        .expect("div should generate a box");
    assert!(matches!(div.outer_display, OuterDisplay::Inline { block: true }));
    assert_eq!(div.inner_formatting_context, FormattingContext::Flow);
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
    let boxes = generate_box_tree(&arena, root_id);
    let div = find(&boxes, &arena, |nt| matches!(nt, NodeType::Div))
        .expect("div should generate a box");
    assert_eq!(div.inner_formatting_context, FormattingContext::Table);
    assert!(matches!(div.outer_display, OuterDisplay::BlockLevel { container: true }));
}

#[test]
fn img_uses_ua_default_inline_block_outer_display() {
    let (root_id, arena) = setup("<img src='x.png'>", "");
    let boxes = generate_box_tree(&arena, root_id);
    let img = find(&boxes, &arena, |nt| matches!(nt, NodeType::Img))
        .expect("<img> should generate a box");
    assert_eq!(img.kind, GeneratedBoxKind::Replaced);
    assert!(
        matches!(img.outer_display, OuterDisplay::Inline { block: true }),
        "<img> UA default should be inline-block, got {:?}",
        img.outer_display
    );
}

#[test]
fn img_explicit_block_outer_overrides_ua_default() {
    let (root_id, arena) = setup("<img src='x.png' style='display: block'>", "");
    let boxes = generate_box_tree(&arena, root_id);
    let img = find(&boxes, &arena, |nt| matches!(nt, NodeType::Img))
        .expect("<img> should generate a box");
    assert_eq!(img.kind, GeneratedBoxKind::Replaced);
    assert!(
        matches!(img.outer_display, OuterDisplay::BlockLevel { container: true }),
        "explicit display: block must win, got {:?}",
        img.outer_display
    );
}

#[test]
fn display_grid_is_grid_formatting_context() {
    let (root_id, arena) = setup(
        "<div style=\"display: grid\">x</div>",
        "",
    );
    let boxes = generate_box_tree(&arena, root_id);
    let div = find(&boxes, &arena, |nt| matches!(nt, NodeType::Div))
        .expect("div should generate a box");
    assert_eq!(div.inner_formatting_context, FormattingContext::Grid);
}

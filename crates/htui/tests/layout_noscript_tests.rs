//! `<noscript>` tests.
//!
//! The engine always runs its script layer, so `<noscript>` mirrors a browser
//! with JavaScript enabled: the element and its content stay in the DOM
//! (addressable by JS / CSS) but never render and never produce layout boxes.

use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::TermLayout;

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn setup(html: &str) -> (NodeId, NodeArena) {
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    if !embedded.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&embedded);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    (root_id, arena)
}

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

/// Layout the document and return the box tree plus the arena.
fn layout_boxes(html: &str) -> (Vec<htui::layout::LayoutBox>, NodeArena) {
    let (root_id, arena) = setup(html);
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes =
        layout_tree(&arena, root_id, containing, &mut cursor, COLS as usize, ROWS as usize);
    (boxes, arena)
}

/// Render the document onto the terminal canvas.
fn draw(html: &str) -> TermLayout {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    tl
}

/// Recursively walk the layout tree (depth-first) and return whether any box
/// belongs to `node_id`.
fn box_for_node<'a>(
    boxes: &'a [htui::layout::LayoutBox],
    node_id: NodeId,
) -> Option<&'a htui::layout::LayoutBox> {
    for b in boxes {
        if b.node_id == node_id {
            return Some(b);
        }
        if let Some(found) = box_for_node(&b.children, node_id) {
            return Some(found);
        }
    }
    None
}

fn find_node_by_type(arena: &NodeArena, target: &NodeType) -> Option<NodeId> {
    arena
        .iter()
        .find(|(_, n)| n.node_type == *target)
        .map(|(id, _)| id)
}

#[test]
fn test_noscript_parses_and_keeps_dom_tree() {
    init();
    // With scripting enabled (this engine's permanent state) a body
    // `<noscript>` follows the generic raw-text algorithm: the element stays
    // in the DOM and its content becomes a single text node, exactly like a
    // browser with JavaScript on.
    let (_, arena) = setup("<div>before</div><noscript><p>fallback</p></noscript>");
    let noscript_id = find_node_by_type(&arena, &NodeType::Noscript)
        .expect("noscript element stays in the DOM");
    let children = arena[noscript_id].children.clone();
    assert_eq!(children.len(), 1, "the content stays as a single child");
    assert_eq!(
        arena[children[0]].node_type,
        NodeType::Text,
        "raw-text parsing keeps the markup as text, like a scripting-enabled browser"
    );
    assert_eq!(
        arena[noscript_id].text_content(&arena),
        "<p>fallback</p>",
        "the raw markup is readable via textContent"
    );
    let parent_id = arena[noscript_id].parent.expect("noscript is nested in the document");
    assert_eq!(
        arena[parent_id].node_type,
        NodeType::Body,
        "noscript is a body-level sibling"
    );
}

#[test]
fn test_noscript_produces_no_boxes() {
    init();
    // The element and its subtree generate no layout boxes — scripting is
    // always enabled in this engine.
    let (boxes, arena) = layout_boxes(
        "<div>shown</div><noscript><p>fallback</p></noscript><div>after</div>",
    );
    let noscript_id = find_node_by_type(&arena, &NodeType::Noscript).expect("noscript in DOM");
    assert!(
        box_for_node(&boxes, noscript_id).is_none(),
        "noscript itself must not produce a box"
    );
    let fallback = arena[noscript_id].children[0];
    assert!(
        box_for_node(&boxes, fallback).is_none(),
        "noscript content must not produce a box"
    );
}

#[test]
fn test_noscript_content_never_paints() {
    init();
    let tl = draw("<div>shown</div><noscript><p>fallback</p></noscript>");
    assert!(
        tl.layout.iter().any(|b| b.word == "s"),
        "sibling content still renders"
    );
    assert!(
        !tl.layout.iter().any(|b| b.word == "f"),
        "noscript content must never paint"
    );
}

#[test]
fn test_noscript_text_nodes_stay_in_dom() {
    init();
    let (_, arena) = setup("<noscript>plain text</noscript>");
    let noscript_id = find_node_by_type(&arena, &NodeType::Noscript).expect("noscript in DOM");
    assert_eq!(
        arena[noscript_id].text_content(&arena),
        "plain text",
        "the raw text stays readable via textContent"
    );
}

#[test]
fn test_noscript_type_selector_parses() {
    init();
    // `noscript` is addressable by CSS type selectors (harmless: it never
    // renders), and the tag table round-trips.
    let (_, arena) = setup("<noscript>fallback</noscript>");
    let noscript_id = find_node_by_type(&arena, &NodeType::Noscript).expect("noscript in DOM");
    assert_eq!(
        arena[noscript_id].node_type.tag_name(),
        Some("noscript"),
        "tag_name round-trips"
    );
    assert_eq!(
        NodeType::from_tag_name("noscript"),
        Some(NodeType::Noscript),
        "from_tag_name round-trips"
    );
}

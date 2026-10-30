//! `<dialog>` and `<template>` tests.
//!
//! `<dialog>` mirrors the browser UA `dialog:not([open]) { display: none }`:
//! closed dialogs produce no box (an author-declared `display` beats the UA
//! default), open dialogs render as a block with the UA default border. A
//! submit button in a `<form method="dialog">` inside a dialog natively
//! closes it (removes `open`) and fires a `close` event.
//!
//! `<template>` holds inert markup: the element and its subtree never render
//! but stay in the DOM (addressable by JS / CSS).

use crossterm::style::Color;
use htui::events::{EventBindings, Geometry, HitMap, HoverState};
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, apply_message};
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::TermLayout;
use std::sync::Arc;

const COLS: u16 = 80;
const ROWS: u16 = 24;

const BOX_H: &str = "─";
const BOX_V: &str = "│";
const BOX_TL: &str = "┌";

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn setup(html: &str, css: &str) -> (NodeId, NodeArena) {
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    // Always run the cascade (even with no stylesheet rules) so inline
    // `style` attributes are applied — the same path the render loop takes.
    let mut css_parser = css_parse::Parser::new(&combined);
    let rules = css_parser.parse();
    css_parse::css_into_node(&mut arena, root_id, &rules);
    (root_id, arena)
}

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

fn layout_boxes(html: &str, css: &str) -> (Vec<htui::layout::LayoutBox>, NodeArena) {
    let (root_id, arena) = setup(html, css);
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes = layout_tree(
        &arena,
        root_id,
        containing,
        &mut cursor,
        COLS as usize,
        ROWS as usize,
    );
    (boxes, arena)
}

fn draw(html: &str, css: &str) -> TermLayout {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html, css);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    tl
}

/// Recursively walk the layout tree (depth-first) and return the first box
/// whose node type matches `target`.
fn find_box_by_type<'a>(
    boxes: &'a [htui::layout::LayoutBox],
    target: &NodeType,
    arena: &NodeArena,
) -> Option<&'a htui::layout::LayoutBox> {
    for b in boxes {
        if &arena[b.node_id].node_type == target {
            return Some(b);
        }
        if let Some(found) = find_box_by_type(&b.children, target, arena) {
            return Some(found);
        }
    }
    None
}

/// Whether any canvas cell carries `word` as its glyph.
fn canvas_has_word(tl: &TermLayout, word: &str) -> bool {
    tl.layout.iter().any(|b| b.word == word)
}

fn find_node_by_type(arena: &NodeArena, target: &NodeType) -> Option<NodeId> {
    arena
        .iter()
        .find(|(_, n)| n.node_type == *target)
        .map(|(id, _)| id)
}

// ============================================================
// <dialog>
// ============================================================

#[test]
fn test_dialog_without_open_renders_nothing() {
    init();
    let (boxes, arena) = layout_boxes("<dialog><p>Hidden</p></dialog>", "");
    assert!(
        find_box_by_type(&boxes, &NodeType::Dialog, &arena).is_none(),
        "closed dialog produces no box"
    );
    assert!(
        find_box_by_type(&boxes, &NodeType::P, &arena).is_none(),
        "closed dialog content produces no box"
    );
    let tl = draw("<dialog><p>Hidden</p></dialog>", "");
    assert!(!canvas_has_word(&tl, "H"), "closed dialog paints nothing");
}

#[test]
fn test_dialog_open_renders_content() {
    init();
    let (boxes, arena) = layout_boxes("<dialog open><p>Shown</p></dialog>", "");
    let dlg =
        find_box_by_type(&boxes, &NodeType::Dialog, &arena).expect("open dialog produces a box");
    assert_eq!(dlg.rect.x, 0, "dialog spans the full width");
    assert_eq!(
        dlg.rect.height, 5,
        "border 1 + <p> UA margins (1 above + 1 below) + 1 content row + border 1"
    );
    assert!(
        find_box_by_type(&boxes, &NodeType::P, &arena).is_some(),
        "dialog content renders when open"
    );
    let tl = draw("<dialog open><p>Shown</p></dialog>", "");
    assert!(canvas_has_word(&tl, "S"), "content paints when open");
}

#[test]
fn test_dialog_ua_border_draws_frame() {
    init();
    // Browser UA default `border: solid` — a one-cell frame around the box.
    let tl = draw("<dialog open>Hello</dialog>", "");
    assert!(canvas_has_word(&tl, BOX_TL), "top-left corner");
    assert!(canvas_has_word(&tl, BOX_V), "left edge");
    assert!(canvas_has_word(&tl, BOX_H), "top edge");
}

#[test]
fn test_dialog_author_border_style_none_removes_ua_border() {
    init();
    // The author's `border-style: none` beats the UA style component: the
    // frame disappears entirely.
    let tl = draw(
        "<dialog open>Hello</dialog>",
        "dialog { border-style: none; }",
    );
    assert!(
        !canvas_has_word(&tl, BOX_TL),
        "no frame with border-style: none"
    );
    let (boxes, arena) = layout_boxes(
        "<dialog open>Hello</dialog>",
        "dialog { border-style: none; }",
    );
    let dlg = find_box_by_type(&boxes, &NodeType::Dialog, &arena).expect("dialog box");
    assert_eq!(dlg.rect.height, 1, "no top/bottom border rows");
}

#[test]
fn test_dialog_author_border_color_applies_keeping_ua_width() {
    init();
    // `border-color` overrides only the color component: the UA width (1) and
    // style (solid) remain, so the frame still draws in the author's color.
    let tl = draw(
        "<dialog open>Hello</dialog>",
        "dialog { border-color: red; }",
    );
    let red = Color::Rgb { r: 255, g: 0, b: 0 };
    let border_color = |tl: &TermLayout| {
        tl.layout
            .iter()
            .find(|b| b.word == BOX_TL)
            .map(|b| b.fcolor)
    };
    assert_eq!(border_color(&tl), Some(Some(red)), "frame draws in red");
    assert!(
        canvas_has_word(&tl, BOX_H),
        "UA width/style keep the frame visible"
    );
}

#[test]
fn test_dialog_author_display_renders_when_closed() {
    init();
    // Author `display` beats the UA `dialog:not([open]) { display: none }`,
    // so an explicit block display renders even without the open attribute.
    let tl = draw("<dialog style=\"display: block\">visible</dialog>", "");
    assert!(
        canvas_has_word(&tl, "v"),
        "author display overrides the closed state"
    );
}

#[test]
fn test_dialog_author_display_none_hides_when_open() {
    init();
    let (boxes, arena) = layout_boxes("<dialog open style=\"display: none\">hidden</dialog>", "");
    assert!(
        find_box_by_type(&boxes, &NodeType::Dialog, &arena).is_none(),
        "display: none wins even over an open dialog"
    );
}

#[test]
fn test_dialog_js_toggle_open_attribute() {
    init();
    // The open attribute is the single source of truth: JS sets / removes it
    // (marking the dialog dirty), and the next layout reflects the change.
    let (mut engine, _geo) = build_geometry(r#"<dialog id="d"><p>Toggle</p></dialog>"#);
    let d = find_node_by_id(&engine.arena(), "d");
    let layout = |engine: &ScriptEngine| {
        let arena = engine.arena();
        let root_id = engine.root_id();
        let containing = BoxConstraint::root(&mock_term_map());
        let mut cursor = Cursor::new();
        layout_tree(
            &arena,
            root_id,
            containing,
            &mut cursor,
            COLS as usize,
            ROWS as usize,
        )
    };
    assert!(
        find_box_by_type(&layout(&engine), &NodeType::Dialog, &engine.arena()).is_none(),
        "closed before the toggle"
    );

    let _ = engine.eval_source("document.getElementById('d').setAttribute('open', '')");
    assert!(engine.arena()[d].attributes.contains_key("open"));
    assert!(
        find_box_by_type(&layout(&engine), &NodeType::Dialog, &engine.arena()).is_some(),
        "open attribute reveals the dialog"
    );

    let _ = engine.eval_source("document.getElementById('d').removeAttribute('open')");
    assert!(!engine.arena()[d].attributes.contains_key("open"));
    assert!(
        find_box_by_type(&layout(&engine), &NodeType::Dialog, &engine.arena()).is_none(),
        "removing open hides the dialog again"
    );
}

// ============================================================
// <dialog> native close (form method=dialog)
// ============================================================

/// The dialog is closed when its `form[method=dialog]` submit button is
/// clicked; the `close` event fires on the dialog afterwards.
fn close_dialog_html(script: &str) -> String {
    format!(
        r#"<script>
          var closes = [];
          var d = document.getElementById('d');
          d.addEventListener('close', function () {{ closes.push('closed'); }});
          globalThis.__closes = closes;
        </script>
        <dialog id="d" open>
          <p>content</p>
          <form method="dialog"><button id="ok">OK</button></form>
        </dialog>
        {script}"#
    )
}

#[test]
fn test_dialog_form_method_dialog_submit_closes_and_fires_close() {
    init();
    let (mut engine, geo) = build_geometry(&close_dialog_html(""));
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("ok").expect("submit button region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let ev = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    let d = find_node_by_id(&engine.arena(), "d");
    assert!(
        !engine.arena()[d].attributes.contains_key("open"),
        "form[method=dialog] submit must close the dialog"
    );
    assert!(ev.dirty.is_some(), "closing marks the dialog dirty");
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__closes.join(','))"),
        "closed",
        "the close event fires on the dialog"
    );
}

#[test]
fn test_dialog_input_submit_closes_and_fires_close() {
    // `<input type=submit>` inside `form[method=dialog]` closes the dialog
    // exactly like a `<button>` submit does (both activate the dialog's
    // default action).
    init();
    let html = r#"<script>
          var closes = [];
          var d = document.getElementById('d');
          d.addEventListener('close', function () { closes.push('closed'); });
          globalThis.__closes = closes;
        </script>
        <dialog id="d" open>
          <form method="dialog"><input id="ok" type="submit"></form>
        </dialog>"#;
    let (mut engine, geo) = build_geometry(html);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("ok").expect("submit input region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    let d = find_node_by_id(&engine.arena(), "d");
    assert!(
        !engine.arena()[d].attributes.contains_key("open"),
        "`<input type=submit>` in form[method=dialog] must close the dialog"
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__closes.join(','))"),
        "closed",
        "the close event fires on the dialog"
    );
}

#[test]
fn test_dialog_onclose_attribute_fires() {
    init();
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          globalThis.__seen = seen;
        </script>
        <dialog id="d" open onclose="seen.push(event.type)">
          <form method="dialog"><button id="ok">OK</button></form>
        </dialog>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("ok").expect("submit button region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "close",
        "onclose attribute runs on the dialog close"
    );
}

#[test]
fn test_dialog_type_button_does_not_close() {
    init();
    let (mut engine, geo) = build_geometry(
        r#"<dialog id="d" open>
          <form method="dialog"><button id="b" type="button">No</button></form>
        </dialog>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("button region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    let d = find_node_by_id(&engine.arena(), "d");
    assert!(
        engine.arena()[d].attributes.contains_key("open"),
        "type=button has no dialog-closing default action"
    );
}

#[test]
fn test_dialog_form_without_method_dialog_does_not_close() {
    init();
    let (mut engine, geo) = build_geometry(
        r#"<dialog id="d" open>
          <form><button id="b">Ok</button></form>
        </dialog>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("button region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    let d = find_node_by_id(&engine.arena(), "d");
    assert!(
        engine.arena()[d].attributes.contains_key("open"),
        "a plain form inside a dialog does not close it"
    );
}

// ============================================================
// <template>
// ============================================================

#[test]
fn test_template_parses_and_keeps_dom_tree() {
    init();
    // The template element and its subtree stay in the DOM (the content is a
    // document fragment by spec), so JS / CSS can still address it.
    let (_, arena) = setup("<div>shown</div><template><p>inert</p></template>", "");
    let t = find_node_by_type(&arena, &NodeType::Template).expect("template in DOM");
    let children = arena[t].children.clone();
    assert_eq!(children.len(), 1, "template keeps its child");
    assert_eq!(
        arena[children[0]].node_type,
        NodeType::P,
        "template content is parsed as an element"
    );
}

#[test]
fn test_template_never_renders() {
    init();
    let (boxes, arena) = layout_boxes("<div>shown</div><template><p>inert</p></template>", "");
    let _t = find_node_by_type(&arena, &NodeType::Template).expect("template in DOM");
    assert!(
        find_box_by_type(&boxes, &NodeType::Template, &arena).is_none(),
        "template itself produces no box"
    );
    assert!(
        find_box_by_type(&boxes, &NodeType::P, &arena).is_none(),
        "template content produces no box"
    );
    assert_eq!(boxes.len(), 1, "only the sibling div lays out");
    let tl = draw("<div>shown</div><template><p>inert</p></template>", "");
    assert!(canvas_has_word(&tl, "s"), "sibling content renders");
    assert!(!canvas_has_word(&tl, "i"), "template content never paints");
}

#[test]
fn test_template_text_content_accessible() {
    init();
    let (_, arena) = setup("<template><p>inert</p></template>", "");
    let t = find_node_by_type(&arena, &NodeType::Template).expect("template in DOM");
    assert_eq!(
        arena[t].text_content(&arena),
        "inert",
        "textContent reads through to the fragment content"
    );
}

#[test]
fn test_template_tag_round_trip() {
    init();
    assert_eq!(
        NodeType::Template.tag_name(),
        Some("template"),
        "tag_name round-trips"
    );
    assert_eq!(
        NodeType::from_tag_name("template"),
        Some(NodeType::Template),
        "from_tag_name round-trips"
    );
    assert_eq!(
        NodeType::Dialog.tag_name(),
        Some("dialog"),
        "dialog tag_name round-trips"
    );
    assert_eq!(
        NodeType::from_tag_name("dialog"),
        Some(NodeType::Dialog),
        "dialog from_tag_name round-trips"
    );
}

// ============================================================
// Helpers
// ============================================================

fn build_geometry(html: &str) -> (ScriptEngine, Geometry) {
    let engine = ScriptEngine::new(html, "", EventBindings::default()).expect("engine initializes");
    let arena = engine.arena();
    let root_id = engine.root_id();
    let term_map = TermMap {
        columns: COLS,
        rows: ROWS,
    };
    let containing = BoxConstraint::root(&term_map);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(
        &arena,
        root_id,
        containing,
        &mut cursor,
        term_map.columns as usize,
        term_map.rows as usize,
    );
    let geo = Geometry {
        arena: Arc::new(arena),
        boxes,
        root_id,
    };
    (engine, geo)
}

fn center(rect: htui::layout::Rect) -> (u32, u32) {
    (
        rect.x as u32 + rect.width.max(1) as u32 / 2,
        rect.y as u32 + rect.height.max(1) as u32 / 2,
    )
}

fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

fn find_node_by_id(arena: &NodeArena, id: &str) -> NodeId {
    arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(nid, _)| nid)
        .expect("element with id exists")
}

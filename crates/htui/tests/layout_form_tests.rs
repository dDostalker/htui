//! Form-control tests — `<input>` `hidden` / `password` / `checkbox` /
//! `radio`.
//!
//! - `type=hidden` never renders a box (the node stays in the DOM).
//! - `type=password` masks every glyph with `•`.
//! - checkbox / radio paint single-cell toggle glyphs; a click toggles
//!   `checked` (radio groups are mutually exclusive by `name`) and fires
//!   `input` / `change` only on actual state flips.

use htui::events::{EventBindings, Geometry, HitMap, interaction::HoverState};
use htui::html::html_parse;
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, Rect, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, apply_message};
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{InputState, RenderCmd, TermLayout, handle_click};

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

fn setup(html: &str, css: &str) -> (NodeId, NodeArena) {
    let mut parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = parser.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    if !combined.trim().is_empty() {
        let mut css_parser = htui::html::css_parse::Parser::new(&combined);
        let rules = css_parser.parse();
        htui::html::css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    (root_id, arena)
}

/// Render the document onto the terminal canvas.
fn draw(html: &str, css: &str) -> (TermMap, TermLayout) {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html, css);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    (term_map, tl)
}

/// Layout the document into a box tree.
fn layout_boxes(html: &str) -> (Vec<htui::layout::LayoutBox>, NodeArena) {
    let (root_id, arena) = setup(html, "");
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes =
        layout_tree(&arena, root_id, containing, &mut cursor, COLS as usize, ROWS as usize);
    (boxes, arena)
}

/// `NodeId` of the element whose `id` attribute equals `id`.
fn find_node_by_id(arena: &NodeArena, id: &str) -> NodeId {
    arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(nid, _)| nid)
        .expect("element with id exists")
}

/// Depth-first search for the first box belonging to `node_id`.
fn find_box<'a>(
    boxes: &'a [htui::layout::LayoutBox],
    node_id: NodeId,
) -> Option<&'a htui::layout::LayoutBox> {
    for b in boxes {
        if b.node_id == node_id {
            return Some(b);
        }
        if let Some(found) = find_box(&b.children, node_id) {
            return Some(found);
        }
    }
    None
}

fn build_geometry(html: &str) -> (ScriptEngine, Geometry) {
    let engine = ScriptEngine::new(html, EventBindings::default()).expect("engine initializes");
    let arena = engine.arena();
    let root_id = engine.root_id();
    let term_map = mock_term_map();
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
        arena,
        boxes,
        root_id,
    };
    (engine, geo)
}

/// A canvas point inside `rect` (its centre).
fn center(rect: Rect) -> (u32, u32) {
    (
        rect.x as u32 + rect.width.max(1) as u32 / 2,
        rect.y as u32 + rect.height.max(1) as u32 / 2,
    )
}

/// Apply one script message like the script thread would, folding a dirty
/// snapshot into a `RenderArena` command.
fn apply(engine: &mut ScriptEngine, map: &HitMap, msg: ScriptMsg) -> Vec<RenderCmd> {
    let mut hover = HoverState::new();
    let ev = apply_message(engine, &mut hover, map, msg);
    let mut cmds = ev.cmds;
    if let Some(d) = ev.dirty {
        cmds.push(RenderCmd::RenderArena {
            arena: d.arena,
            root_id: d.root_id,
            scroll_y: 0,
            zoom: 1.0,
            hovered: ev.hovered,
            dirty: d.dirty,
        });
    }
    cmds
}

/// Click at canvas coordinates through the real input-loop click handler.
fn click(state: &mut InputState, engine: &mut ScriptEngine, map: &HitMap, x: u32, y: u32) {
    let arena = engine.arena();
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_click(state, map, &arena, x, y, &mut msgs, &mut cmds);
    for msg in msgs {
        apply(engine, map, msg);
    }
}

/// Evaluate a JS expression as a string.
fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

// ============================================================
// hidden
// ============================================================

#[test]
fn test_hidden_input_produces_no_box() {
    // `<input type=hidden>` renders nothing (like `display: none`) while
    // surrounding text keeps flowing.
    let (boxes, arena) = layout_boxes(r#"<div id="wrap">a<input id="h" type="hidden" value="x">b</div>"#);
    let h = find_node_by_id(&arena, "h");
    assert!(
        find_box(&boxes, h).is_none(),
        "hidden input must not produce a layout box"
    );

    let (term_map, tl) = draw(r#"<div>a<input type="hidden" value="x">b</div>"#, "");
    let cols = term_map.columns as usize;
    let joined: String = tl
        .layout
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.word.is_empty() && b.word != " " && b.word != "[")
        .map(|(i, b)| format!("{}{}", b.word, if i % cols == cols - 1 { " " } else { "" }))
        .collect();
    assert!(
        joined.contains("ab"),
        "text around the hidden input must still render (got {joined:?})"
    );
}

#[test]
fn test_hidden_input_stays_in_dom() {
    let (_, arena) = setup(
        r#"<form><input id="h" type="hidden" name="tok" value="abc"></form>"#,
        "",
    );
    let h = find_node_by_id(&arena, "h");
    let n = &arena[h];
    assert_eq!(n.node_type, NodeType::Input, "hidden input stays a DOM node");
    assert_eq!(
        n.attributes.get("value").map(String::as_str),
        Some("abc"),
        "form data survives"
    );
}

// ============================================================
// password
// ============================================================

#[test]
fn test_password_input_masks_value() {
    let (_, tl) = draw(r#"<input type="password" value="secret">"#, "");
    let bullets = tl
        .layout
        .iter()
        .filter(|b| b.word == "\u{2022}")
        .count();
    assert_eq!(bullets, 6, "every password character must be masked as `•`");
    assert!(
        !tl
            .layout
            .iter()
            .any(|b| b.word == "s" || b.word == "e" || b.word == "c" || b.word == "r" || b.word == "t"),
        "the plaintext must never be painted"
    );
}

// ============================================================
// checkbox / radio rendering
// ============================================================

#[test]
fn test_checkbox_unchecked_renders_box_glyph() {
    let (boxes, arena) = layout_boxes(r#"<div><input id="c" type="checkbox"></div>"#);
    let c = find_node_by_id(&arena, "c");
    let b = find_box(&boxes, c).expect("checkbox box");
    assert_eq!(b.rect.width, 1, "checkbox is a single cell");

    let (_, tl) = draw(r#"<input type="checkbox">"#, "");
    assert!(
        tl.layout.iter().any(|b| b.word == "\u{2610}"),
        "unchecked checkbox must paint `☐`"
    );
    assert!(
        !tl.layout.iter().any(|b| b.word == "[" || b.word == "]"),
        "toggle controls must not draw the text-field frame"
    );
}

#[test]
fn test_checkbox_checked_renders_box_glyph() {
    let (_, tl) = draw(r#"<input type="checkbox" checked>"#, "");
    assert!(
        tl.layout.iter().any(|b| b.word == "\u{2611}"),
        "checked checkbox must paint `☑`"
    );
}

#[test]
fn test_radio_renders_circle_glyphs() {
    let (_, tl) = draw(r#"<input type="radio"><input type="radio" checked>"#, "");
    assert!(
        tl.layout.iter().any(|b| b.word == "\u{25CB}"),
        "unchecked radio must paint `○`"
    );
    assert!(
        tl.layout.iter().any(|b| b.word == "\u{25C9}"),
        "checked radio must paint `◉`"
    );
}

// ============================================================
// checkbox / radio interaction
// ============================================================

#[test]
fn test_checkbox_toggle_flips_checked_attribute() {
    let (mut engine, geo) = build_geometry(r#"<div><input id="c" type="checkbox"></div>"#);
    let map = HitMap::build(&geo);
    let c = find_node_by_id(&engine.arena(), "c");
    assert!(
        !engine.arena().get(c).unwrap().attributes.contains_key("checked"),
        "starts unchecked"
    );

    let cmds = apply(&mut engine, &map, ScriptMsg::ToggleCheck { node_id: c });
    assert!(
        engine.arena().get(c).unwrap().attributes.contains_key("checked"),
        "first click checks the box"
    );
    assert!(
        cmds.iter().any(|cmd| matches!(cmd, RenderCmd::RenderArena { .. })),
        "toggle must mark the node dirty for re-render"
    );

    apply(&mut engine, &map, ScriptMsg::ToggleCheck { node_id: c });
    assert!(
        !engine.arena().get(c).unwrap().attributes.contains_key("checked"),
        "second click unchecks the box"
    );
}

#[test]
fn test_radio_group_mutual_exclusion() {
    // Two radios share a `name`: checking one clears the other.
    let (mut engine, geo) = build_geometry(
        r#"<div><input id="a" type="radio" name="g" checked><input id="b" type="radio" name="g"></div>"#,
    );
    let map = HitMap::build(&geo);
    let a = find_node_by_id(&engine.arena(), "a");
    let b = find_node_by_id(&engine.arena(), "b");
    assert!(engine.arena().get(a).unwrap().attributes.contains_key("checked"));

    apply(&mut engine, &map, ScriptMsg::ToggleCheck { node_id: b });

    let arena = engine.arena();
    assert!(
        !arena.get(a).unwrap().attributes.contains_key("checked"),
        "same-name radio sibling must be cleared"
    );
    assert!(
        arena.get(b).unwrap().attributes.contains_key("checked"),
        "clicked radio must be checked"
    );
}

#[test]
fn test_radio_reclick_keeps_selection() {
    // Clicking an already-selected radio is a no-op in browsers.
    let (mut engine, geo) = build_geometry(
        r#"<div><input id="a" type="radio" name="g" checked></div>"#,
    );
    let map = HitMap::build(&geo);
    let a = find_node_by_id(&engine.arena(), "a");
    apply(&mut engine, &map, ScriptMsg::ToggleCheck { node_id: a });
    assert!(
        engine.arena().get(a).unwrap().attributes.contains_key("checked"),
        "radio stays selected on re-click"
    );
}

#[test]
fn test_checkbox_click_full_path_toggles_and_fires_change() {
    // End-to-end: click through the real input-loop handler, the toggle is
    // applied on the script thread and `change` fires (once per flip).
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          document.getElementById('c').addEventListener('change', function () {
            seen.push('change');
          });
          globalThis.__seen = seen;
        </script>
        <div><input id="c" type="checkbox"></div>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("c").expect("checkbox region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    let c = find_node_by_id(&engine.arena(), "c");
    assert!(
        engine.arena().get(c).unwrap().attributes.contains_key("checked"),
        "click must check the box"
    );
    assert_eq!(
        eval_str(&mut engine, "globalThis.__seen.length"),
        "1",
        "first flip fires one change"
    );

    click(&mut state, &mut engine, &map, x, y);
    assert!(
        !engine.arena().get(c).unwrap().attributes.contains_key("checked"),
        "second click unchecks"
    );
    assert_eq!(
        eval_str(&mut engine, "globalThis.__seen.length"),
        "2",
        "second flip fires another change"
    );
}

#[test]
fn test_radio_click_fires_change_once_for_group_flip() {
    // Checking one radio clears the sibling and fires `change` once — the
    // clicked member flips, the sibling merely loses its attribute.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          document.getElementById('b').addEventListener('change', function () {
            seen.push('b');
          });
          globalThis.__seen = seen;
        </script>
        <div>
          <input id="a" type="radio" name="g" checked>
          <input id="b" type="radio" name="g">
        </div>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("radio region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__seen.length"),
        "1",
        "checking a radio fires change on it"
    );
}

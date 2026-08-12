//! Form-control tests — `<input>` `hidden` / `password` / `checkbox` /
//! `radio`, `<label>` delegation, `<button>` and `<form>` submit/reset
//! events.
//!
//! - `type=hidden` never renders a box (the node stays in the DOM).
//! - `type=password` masks every glyph with `•`.
//! - checkbox / radio paint single-cell toggle glyphs; a click toggles
//!   `checked` (radio groups are mutually exclusive by `name`) and fires
//!   `input` / `change` only on actual state flips.
//! - `<label>` is inline and owns a hit region over its text; a click
//!   delegates to the control named by `for` (or the first wrapped
//!   labelable element).
//! - `<button>` paints `[ text ]`; a `disabled` button swallows clicks and
//!   `type=submit` / `reset` fire the matching event on the containing
//!   `<form>` (the engine never navigates or resets controls itself).

use htui::events::{EventBindings, Geometry, HitMap, interaction::HoverState};
use htui::html::css_parse::DynamicState;
use htui::html::html_parse;
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, Rect, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, apply_message};
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{InputState, RenderCmd, TermLayout, handle_click, paste_into_field};

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
            dynamic: DynamicState {
                hovered: ev.hovered,
                focused: None,
            },
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
    assert_eq!(b.rect.width, 2, "checkbox is glyph + one spacing cell");

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
// bracketed paste
// ============================================================

#[test]
fn test_paste_into_single_line_input_collapses_newlines() {
    // Bracketed paste into a focused `<input>`: `\r` / `\n` collapse so the
    // single-line field never shows a raw newline.
    let mut state = InputState::new();
    state.focused_input = Some(NodeId::default());
    state.focused_is_textarea = false;
    state.input_value = "ab".to_string();
    state.input_cursor = 2;
    let mut cmds = Vec::new();
    assert!(paste_into_field(&mut state, "xy\nz", NodeId::default(), &mut cmds));
    assert_eq!(state.input_value, "abxyz");
    assert_eq!(state.input_cursor, 5, "caret advances past the pasted text");
    assert!(
        cmds.iter().any(|c| matches!(c, RenderCmd::RenderInput { .. })),
        "paste must repaint the field"
    );
}

#[test]
fn test_paste_into_textarea_keeps_newlines() {
    let mut state = InputState::new();
    state.focused_input = Some(NodeId::default());
    state.focused_is_textarea = true;
    state.input_value = "ab".to_string();
    state.input_cursor = 2;
    let mut cmds = Vec::new();
    assert!(paste_into_field(&mut state, "xy\nz", NodeId::default(), &mut cmds));
    assert_eq!(state.input_value, "abxy\nz", "textarea keeps pasted newlines");
    assert_eq!(state.input_cursor, 6);
}

#[test]
fn test_paste_at_caret_inserts_in_middle() {
    let mut state = InputState::new();
    state.focused_input = Some(NodeId::default());
    state.input_value = "abcd".to_string();
    state.input_cursor = 2;
    let mut cmds = Vec::new();
    assert!(paste_into_field(&mut state, "XY", NodeId::default(), &mut cmds));
    assert_eq!(state.input_value, "abXYcd");
    assert_eq!(state.input_cursor, 4);
}

#[test]
fn test_paste_only_newlines_into_input_is_noop() {
    let mut state = InputState::new();
    state.input_value = "ab".to_string();
    state.input_cursor = 1;
    state.focused_is_textarea = false;
    let mut cmds = Vec::new();
    assert!(
        !paste_into_field(&mut state, "\r\n", NodeId::default(), &mut cmds),
        "all-collapsed paste must not touch the value"
    );
    assert_eq!(state.input_value, "ab");
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

// ============================================================
// label
// ============================================================

#[test]
fn test_label_renders_inline_with_surrounding_text() {
    // `<label>` is an inline container: its text flows on the same line as
    // the surrounding paragraph text and it has no layout box of its own.
    let (boxes, arena) = layout_boxes(r#"<p>fo<label id="l">la</label></p>"#);
    let l = find_node_by_id(&arena, "l");
    assert!(find_box(&boxes, l).is_none(), "inline label must not produce a box");

    let (term_map, tl) = draw(r#"<p>fo<label id="l">la</label></p>"#, "");
    let cols = term_map.columns as usize;
    let joined: String = tl
        .layout
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.word.is_empty() && b.word != " " && b.word != "[")
        .map(|(i, b)| format!("{}{}", b.word, if i % cols == cols - 1 { " " } else { "" }))
        .collect();
    assert!(
        joined.contains("fola"),
        "label text must flow inline with the paragraph (got {joined:?})"
    );
}

#[test]
fn test_label_owns_hit_region_over_its_text() {
    // The label's text slice gets its own region so a click resolves to the
    // label (the delegation target), not the surrounding block.
    let (_, geo) = build_geometry(r#"<p>fo<label id="l">la</label></p>"#);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("l").expect("label hit region exists");
    assert_eq!(region.rect.width, 2, "region covers the label's `la` text slice");
    assert_eq!(region.rect.height, 1);
}

#[test]
fn test_label_for_click_focuses_input() {
    let (mut engine, geo) = build_geometry(
        r#"<label id="l" for="f">Name:</label><input id="f" type="text">"#,
    );
    let map = HitMap::build(&geo);
    let f = find_node_by_id(&engine.arena(), "f");
    let region = map.resolve_by_id("l").expect("label hit region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        state.focused_input,
        Some(f),
        "clicking `<label for>` text must focus the associated input"
    );
}

#[test]
fn test_label_wrapping_checkbox_toggles_on_click() {
    let (mut engine, geo) = build_geometry(
        r#"<label id="l"><input id="c" type="checkbox"> Agree</label>"#,
    );
    let map = HitMap::build(&geo);
    let c = find_node_by_id(&engine.arena(), "c");
    let region = map.resolve_by_id("l").expect("label hit region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert!(
        engine.arena().get(c).unwrap().attributes.contains_key("checked"),
        "clicking a wrapping label must activate its control"
    );
}

#[test]
fn test_label_wrapped_checkbox_renders_box() {
    // A control inside a wrapping `<label>` keeps its own box (the label is
    // an inline container — its text flows into the parent, the replaced
    // element is laid out below the text line like any inline-level child).
    let (boxes, arena) =
        layout_boxes(r#"<p><label><input id="c" type="checkbox"> Agree</label></p>"#);
    let c = find_node_by_id(&arena, "c");
    let b = find_box(&boxes, c).expect("wrapped checkbox must keep its box");
    assert_eq!(b.rect.width, 2, "toggle control is glyph + one spacing cell");

    let (_, tl) = draw(r#"<p><label><input type="checkbox"> Agree</label></p>"#, "");
    assert!(
        tl.layout.iter().any(|b| b.word == "\u{2610}"),
        "wrapped checkbox must paint `☐`"
    );
}

// ============================================================
// button
// ============================================================

#[test]
fn test_button_renders_bracketed_label() {
    let (_, tl) = draw(r#"<button>Save</button>"#, "");
    let glyphs: String = tl
        .layout
        .iter()
        .filter(|b| !b.word.is_empty() && b.word != " ")
        .map(|b| b.word.clone())
        .collect();
    assert_eq!(glyphs, "[Save]", "button must paint `[ text ]` frame");
}

#[test]
fn test_disabled_button_swallows_click() {
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('b').addEventListener('click', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <button id="b" disabled>No</button>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("button hit region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "0",
        "a disabled button must swallow the click entirely"
    );
}

// ============================================================
// form submit / reset
// ============================================================

#[test]
fn test_submit_button_fires_submit_on_containing_form() {
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('submit', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><button id="b">Go</button></form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("button hit region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "1",
        "clicking a submit button must fire `submit` on the containing form"
    );
}

#[test]
fn test_onsubmit_attribute_runs_on_submit_click() {
    let (mut engine, geo) = build_geometry(
        r#"<form id="f" onsubmit="globalThis.__s = (globalThis.__s || 0) + 1">
             <button id="b">Go</button>
           </form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("button hit region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__s"),
        "1",
        "the `onsubmit` attribute handler must run on submit"
    );
}

#[test]
fn test_reset_button_fires_reset_on_containing_form() {
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('reset', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><button id="b" type="reset">Clear</button></form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("button hit region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "1",
        "clicking a reset button must fire `reset` on the containing form"
    );
}

#[test]
fn test_type_button_does_not_submit() {
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('submit', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><button id="b" type="button">Go</button></form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("button hit region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "0",
        "`type=button` must not fire submit"
    );
}

#[test]
fn test_submit_button_nested_in_div_finds_ancestor_form() {
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('submit', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><div><section><button id="b">Go</button></section></div></form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("b").expect("button hit region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "1",
        "a submit button nested under divs must still find its ancestor form"
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

// ============================================================
// `<input type=button|submit|reset>` — push-button inputs
// ============================================================

#[test]
fn test_input_submit_fires_submit_on_containing_form() {
    // `<input type=submit>` behaves like `<button type=submit>`: a click fires
    // `submit` on the containing form.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('submit', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><input id="s" type="submit"></form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("submit input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "1",
        "clicking `<input type=submit>` must fire `submit` on the containing form"
    );
}

#[test]
fn test_input_reset_fires_reset_on_containing_form() {
    // `<input type=reset>` fires `reset` on the containing form.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('reset', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><input id="s" type="reset"></form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("reset input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "1",
        "clicking `<input type=reset>` must fire `reset` on the containing form"
    );
}

#[test]
fn test_input_type_button_does_not_submit() {
    // `<input type=button>` is inert: no submit, no reset.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('submit', function () { n.push(1); });
          document.getElementById('f').addEventListener('reset', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><input id="s" type="button" value="Go"></form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("button input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "0",
        "`type=button` input must not fire submit or reset"
    );
}

#[test]
fn test_input_submit_click_does_not_focus() {
    // A button input activates on click but never takes text focus.
    let (mut engine, geo) = build_geometry(
        r#"<input id="f" type="text"><input id="s" type="submit">"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("submit input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    state.focused_input = Some(find_node_by_id(&engine.arena(), "f"));
    click(&mut state, &mut engine, &map, x, y);
    assert!(
        state.focused_input.is_none(),
        "clicking a submit input must blur any focused text field"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('s').getAttribute('type')"),
        "submit",
        "the button input stays a submit input"
    );
}

// ============================================================
// `<input type=image>` / Enter implicit submission / readonly
// ============================================================

#[test]
fn test_input_image_fires_submit_on_containing_form() {
    // `<input type=image>` is a submit button: a click fires `submit`.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('submit', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><input id="s" type="image" alt="Go"></form>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("image input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    click(&mut state, &mut engine, &map, x, y);
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "1",
        "clicking `<input type=image>` must fire `submit` on the containing form"
    );
}

#[test]
fn test_enter_implicit_submission_fires_submit_on_form() {
    // Pressing Enter in a focused text field submits its containing form —
    // the browser's implicit submission, routed as `ScriptMsg::FormSubmit`.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var n = [];
          document.getElementById('f').addEventListener('submit', function () { n.push(1); });
          globalThis.__n = n;
        </script>
        <form id="f"><input id="t" type="text"></form>"#,
    );
    let map = HitMap::build(&geo);
    let form = find_node_by_id(&engine.arena(), "f");
    let cmds = apply(&mut engine, &map, ScriptMsg::FormSubmit { form_id: form });
    assert_eq!(
        eval_str(&mut engine, "globalThis.__n.length"),
        "1",
        "Enter must submit the containing form"
    );
    assert!(!cmds.is_empty() || eval_str(&mut engine, "globalThis.__n.length") == "1");
}

#[test]
fn test_readonly_input_focuses_but_disabled_does_not() {
    // `readonly` keeps the field focusable (unlike `disabled`, which swallows
    // the click entirely) — matching the browser: readonly text is selectable
    // and copyable, just not editable.
    let (mut engine, geo) = build_geometry(
        r#"<input id="r" type="text" value="fixed" readonly><input id="d" type="text" disabled>"#,
    );
    let map = HitMap::build(&geo);

    // Clicking the readonly field focuses it.
    let mut state = InputState::new();
    let region = map.resolve_by_id("r").expect("readonly region");
    let (x, y) = center(region.rect);
    click(&mut state, &mut engine, &map, x, y);
    let r = find_node_by_id(&engine.arena(), "r");
    assert_eq!(
        state.focused_input,
        Some(r),
        "readonly fields stay focusable"
    );

    // Clicking the disabled field swallows the click: no focus change.
    let region = map.resolve_by_id("d").expect("disabled region");
    let (x, y) = center(region.rect);
    click(&mut state, &mut engine, &map, x, y);
    let d = find_node_by_id(&engine.arena(), "d");
    assert_eq!(
        state.focused_input,
        Some(r),
        "disabled fields must never take focus"
    );
    assert_ne!(state.focused_input, Some(d));
}

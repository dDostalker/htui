//! `<input>` field tests — replaced-element rendering, the pure text-editing
//! logic (`apply_input_key`), and the engine edit-state rendering path.
//!
//! The editing tests drive `apply_input_key` directly (no terminal, no event
//! loop); the render tests draw a small document and assert on the canvas.

use crossterm::event::KeyCode;
use htui::events::EventBindings;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::script::ScriptEngine;
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{InputRenderState, TermLayout, apply_input_key, selection_range};
use htui::tui::graph_api::Block;

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

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

/// Render and return the per-row text strings for inspection.
fn render_rows(html: &str) -> (Vec<String>, TermLayout) {
    let (root_id, arena) = setup(html);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let cols = term_map.columns as usize;
    let mut rows: Vec<String> = Vec::new();
    for chunk in tl.layout.chunks(cols) {
        let s: String = chunk.iter().map(|b| b.word.as_str()).collect();
        rows.push(s);
    }
    (rows, tl)
}

#[test]
fn select_renders_as_frame_with_current_label() {
    init();
    // A `<select>` draws a bracketed field showing the option that matches
    // the `value` attribute (falling back to the first option).
    let (rows, _) = render_rows(
        r#"<div><select value="b"><option>A</option><option value="b">BB</option></select></div>"#,
    );
    let r = &rows[0];
    assert!(r.contains("[BB"), "select must show the selected option; row={r:?}");
}

#[test]
fn select_renders_first_option_without_value() {
    init();
    // No `value` attribute → the first option is shown.
    let (rows, _) = render_rows(
        r#"<div><select><option>Alpha</option><option>Beta</option></select></div>"#,
    );
    let r = &rows[0];
    assert!(r.contains("[Alpha"), "select must fall back to the first option; row={r:?}");
}

/// Find the first node with `node_type == NodeType::Input`.
fn find_input(arena: &NodeArena, root: NodeId) -> NodeId {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        if node.node_type == NodeType::Input {
            return id;
        }
        stack.extend(node.children.iter().copied());
    }
    panic!("no <input> in document");
}

/// Find the first node with `node_type == NodeType::Input` in a live-engine
/// arena (mirrors [`find_input`] for standalone arenas).
fn find_input_in_arena(arena: &NodeArena, root: NodeId) -> NodeId {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        if node.node_type == NodeType::Input {
            return id;
        }
        stack.extend(node.children.iter().copied());
    }
    panic!("no <input> in document");
}

#[test]
fn selection_range_orders_anchor_and_caret() {
    // Anchor may sit on either side of the caret; the range is always ordered.
    assert_eq!(selection_range(1, 4), (1, 4));
    assert_eq!(selection_range(4, 1), (1, 4));
    assert_eq!(selection_range(2, 2), (2, 2));
}

#[test]
fn input_renders_selection_highlight() {
    init();
    let (root_id, arena) = setup(r#"<div><input></div>"#);
    let input_id = find_input(&arena, root_id);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.input_states.insert(
        input_id,
        InputRenderState {
            value: "hello".to_string(),
            cursor: 4,
            focused: true,
            selection: Some((1, 4)),
        },
    );
    tl.draw(&arena, root_id, &term_map).unwrap();
    // Cells: `[h e l l o · ]`. Selection (1..4) = `ell` (cols 2-4) get the
    // Tokyo Night selection blue; the caret sits on `o` (inverted) and the
    // unselected `h` keeps the field's (none) colours.
    let cell = |x: usize| -> &Block { &tl.layout[x] };
    assert_eq!(cell(1).word, "h");
    assert_eq!(cell(1).bcolor, None, "unselected char keeps default bg");
    assert_eq!(
        cell(2).bcolor,
        Some(crossterm::style::Color::Rgb {
            r: 0x3b,
            g: 0x42,
            b: 0x61,
        }),
        "selected char gets the highlight bg"
    );
    assert_eq!(cell(4).bcolor, cell(2).bcolor, "selection is contiguous");
    assert_eq!(
        cell(5).bcolor,
        Some(crossterm::style::Color::White),
        "caret cell stays inverted"
    );
}

#[test]
fn input_renders_empty_field_without_panicking() {
    init();
    // An `<input>` with no `value` renders an empty bracketed field — the
    // caret window must not index past the empty character vector.
    let (rows, _) = render_rows(r#"<div><input></div>"#);
    assert_eq!(rows[0].chars().nth(0), Some('['));
    assert_eq!(rows[0].chars().nth(7), Some(']'));
}

#[test]
fn input_long_value_windows_around_the_caret() {
    init();
    // Width 8 (6 content cells), value much longer: the visible window must
    // follow the caret instead of blanking the field (regression: the window
    // used to be positioned in absolute value columns, so a long value drew
    // nothing at all).
    let (root_id, arena) = setup(r#"<div><input width="8"></div>"#);
    let input_id = find_input(&arena, root_id);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.input_states.insert(
        input_id,
        InputRenderState {
            value: "abcdefghijkl".to_string(),
            cursor: 12,
            focused: true,
            selection: None,
        },
    );
    tl.draw(&arena, root_id, &term_map).unwrap();
    // Caret at the end (a blank inverted cell): the window follows the caret
    // and shows `hijkl` (indices 7..12 of "abcdefghijkl") + the caret cell,
    // with the right frame intact.
    let row: String = tl.layout.iter().take(8).map(|b| b.word.as_str()).collect();
    assert_eq!(row, "[hijkl ]");
}

#[test]
fn input_renders_multibyte_values_at_display_width() {
    init();
    let (rows, _) = render_rows(r#"<div><input value="你好"></div>"#);
    // CJK glyphs occupy two columns each: the renderer skips their second
    // (empty-word) cells, so the field reads `[你好  ]` and the right frame
    // bracket stays intact.
    let r = &rows[0];
    assert_eq!(r.chars().nth(0), Some('['));
    assert_eq!(r.chars().nth(1), Some('你'));
    assert_eq!(r.chars().nth(2), Some('好'));
    assert_eq!(r.chars().nth(3), Some(' '), "padding after the wide glyphs");
    assert_eq!(r.chars().nth(5), Some(']'), "right frame must survive wide glyphs");
}

#[test]
fn set_value_commits_edited_value_to_the_dom_and_marks_dirty() {
    // The blur path writes the edited value back into the DOM `value`
    // attribute (single source of truth) and marks the node dirty, so the
    // commit always reaches the render thread as a `RenderArena` — a JS blur
    // handler that rewrites the value (e.g. clearing a submitted box) is what
    // ends up on screen. Committing an unchanged value is a no-op.
    init();
    let mut engine = ScriptEngine::new(
        r#"<div><input value="initial"></div>"#,
        EventBindings::default(),
    )
    .expect("engine");
    let input_id = find_input_in_arena(&engine.arena(), engine.root_id());

    engine.set_value(input_id, "edited");

    assert!(
        engine.take_dirty().nodes.contains(&input_id),
        "set_value must mark the node dirty"
    );
    let stored = engine
        .arena()
        .get(input_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned();
    assert_eq!(stored.as_deref(), Some("edited"));

    engine.set_value(input_id, "edited");
    assert!(
        engine.take_dirty().is_empty(),
        "committing the same value again must not mark dirty"
    );
}

#[test]
fn layout_drops_unfocused_input_state_but_keeps_focused() {
    init();
    let (root_id, arena) = setup(r#"<div><input value="hi"></div>"#);
    let input_id = find_input(&arena, root_id);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);

    // A stale unfocused cache entry (the transient blur repaint) must not
    // survive a fresh layout: the arena's `value` attribute is the display
    // truth for unfocused fields.
    tl.input_states.insert(
        input_id,
        InputRenderState {
            value: "stale".to_string(),
            cursor: 0,
            focused: false,
            selection: None,
        },
    );
    tl.draw(&arena, root_id, &term_map).unwrap();
    assert!(
        !tl.input_states.contains_key(&input_id),
        "layout must drop stale unfocused entries"
    );
    let row: String = tl.layout.iter().take(8).map(|b| b.word.as_str()).collect();
    assert_eq!(row, "[hi    ]", "unfocused field paints the DOM value");

    // A focused entry holds the live edit buffer and must survive layout.
    tl.input_states.insert(
        input_id,
        InputRenderState {
            value: "typing".to_string(),
            cursor: 5,
            focused: true,
            selection: None,
        },
    );
    tl.draw(&arena, root_id, &term_map).unwrap();
    assert!(
        tl.input_states.contains_key(&input_id),
        "focused edit state must survive layout"
    );
    let row: String = tl.layout.iter().take(8).map(|b| b.word.as_str()).collect();
    assert_eq!(row, "[typing]", "focused field paints the live edit buffer");
}

#[test]
fn input_renders_as_bracketed_field_with_value() {
    init();
    let (rows, _) = render_rows(r#"<div><input value="hi"></div>"#);
    // Default field width = value + 2, clamped to a tappable minimum of 8.
    assert_eq!(rows[0].chars().nth(0), Some('['));
    assert_eq!(rows[0].chars().nth(1), Some('h'));
    assert_eq!(rows[0].chars().nth(2), Some('i'));
    assert_eq!(rows[0].chars().nth(7), Some(']'), "field must close at col 7");
    assert_eq!(rows[0].chars().nth(8), Some(' '), "field must not overflow");
}

#[test]
fn input_respects_width_attribute() {
    init();
    let (rows, _) = render_rows(r#"<div><input width="4" value="abc"></div>"#);
    // width=4 cells: the frame brackets + 2 content cells. The caret starts
    // at position 0, so the window shows `ab`.
    assert_eq!(rows[0].chars().nth(0), Some('['));
    assert_eq!(rows[0].chars().nth(1), Some('a'));
    assert_eq!(rows[0].chars().nth(2), Some('b'));
    assert_eq!(rows[0].chars().nth(3), Some(']'), "close at width-1");
}

#[test]
fn apply_input_key_inserts_removes_and_moves() {
    let mut value = String::from("hello");
    let mut cursor = 5;

    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Char('!')));
    assert_eq!(value, "hello!");
    assert_eq!(cursor, 6);

    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Backspace));
    assert_eq!(value, "hello");
    assert_eq!(cursor, 5);

    // Left + insert in the middle.
    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Left));
    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Char('X')));
    assert_eq!(value, "hellXo");
    assert_eq!(cursor, 5);

    // Delete removes the char right of the caret.
    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Delete));
    assert_eq!(value, "hellX");
}

#[test]
fn apply_input_key_home_end_and_limits() {
    let mut value = String::from("ab");
    let mut cursor = 2;

    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Home));
    assert_eq!(cursor, 0);
    // At the start, Left / Backspace are no-ops.
    assert!(!apply_input_key(&mut value, &mut cursor, KeyCode::Left));
    assert!(!apply_input_key(&mut value, &mut cursor, KeyCode::Backspace));

    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::End));
    assert_eq!(cursor, 2);
    assert!(!apply_input_key(&mut value, &mut cursor, KeyCode::Right));
    assert!(!apply_input_key(&mut value, &mut cursor, KeyCode::Delete));

    // Enter / Esc are not consumed by the editor.
    assert!(!apply_input_key(&mut value, &mut cursor, KeyCode::Enter));
    assert!(!apply_input_key(&mut value, &mut cursor, KeyCode::Esc));
}

#[test]
fn apply_input_key_handles_multibyte_chars() {
    // Caret is a character index; multi-byte characters must stay intact.
    let mut value = String::from("你好");
    let mut cursor = 0;

    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Char('!')));
    assert_eq!(value, "!你好");
    assert_eq!(cursor, 1);

    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Backspace));
    assert_eq!(value, "你好");
    assert_eq!(cursor, 0);

    // Move to the end and delete the last (multi-byte) char.
    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::End));
    assert!(apply_input_key(&mut value, &mut cursor, KeyCode::Backspace));
    assert_eq!(value, "你");
    assert_eq!(cursor, 1);
}

#[test]
fn engine_input_state_renders_value_and_caret() {
    init();
    let (root_id, arena) = setup(r#"<div><input value="initial"></div>"#);
    let input_id = find_input(&arena, root_id);

    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.input_states.insert(
        input_id,
        InputRenderState {
            value: "abc".to_string(),
            cursor: 1,
            focused: true,
            selection: None,
        },
    );
    tl.draw(&arena, root_id, &term_map).unwrap();

    // Field shows the engine value `abc`, caret inverts the char at index 1.
    let cell = |x: usize| -> &Block { &tl.layout[x] };
    assert_eq!(cell(0).word, "[");
    assert_eq!(cell(1).word, "a");
    assert_eq!(cell(2).word, "b");
    assert_eq!(cell(3).word, "c");
    // Default field width is fixed at 8 cells, independent of the value.
    assert_eq!(cell(7).word, "]");
    // No CSS colours on the input: plain cells have no fg / bg, while the
    // caret cell (char index 1 → column 2) is inverted to black-on-white.
    assert_eq!(cell(1).fcolor, None);
    assert_eq!(cell(1).bcolor, None);
    assert_eq!(cell(2).bcolor, Some(crossterm::style::Color::White));
}

//! `<input>` field tests — replaced-element rendering, the pure text-editing
//! logic (`apply_input_key`), and the engine edit-state rendering path.
//!
//! The editing tests drive `apply_input_key` directly (no terminal, no event
//! loop); the render tests draw a small document and assert on the canvas.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use htui::events::EventBindings;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::script::ScriptEngine;
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{
    InputRenderState, InputState, TermLayout, apply_input_key, apply_input_word_key,
    collect_input_ids, is_input_field_key, key_event_info, next_word_boundary, number_input_allows_char,
    prev_word_boundary, selection_range,
};
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
    tl.draw(&arena, root_id, &term_map);
    let cols = term_map.columns as usize;
    let mut rows: Vec<String> = Vec::new();
    for chunk in tl.layout.chunks(cols) {
        let s: String = chunk.iter().map(|b| b.word.as_str()).collect();
        rows.push(s);
    }
    (rows, tl)
}

/// Layout the document into a box tree (for geometry assertions).
fn layout_boxes(arena: &NodeArena, root: NodeId) -> Vec<htui::layout::LayoutBox> {
    use htui::layout::{BoxConstraint, Cursor, layout_tree};
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    layout_tree(arena, root, containing, &mut cursor, COLS as usize, ROWS as usize)
}

/// Depth-first search for the first box belonging to `node_id`.
fn find_box(
    boxes: &[htui::layout::LayoutBox],
    node_id: NodeId,
) -> Option<&htui::layout::LayoutBox> {
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
    tl.draw(&arena, root_id, &term_map);
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
    assert_eq!(rows[0].chars().next(), Some('['));
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
    tl.draw(&arena, root_id, &term_map);
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
    assert_eq!(r.chars().next(), Some('['));
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
    let input_id = find_input(&engine.arena(), engine.root_id());

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
    tl.draw(&arena, root_id, &term_map);
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
    tl.draw(&arena, root_id, &term_map);
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
    assert_eq!(rows[0].chars().next(), Some('['));
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
    assert_eq!(rows[0].chars().next(), Some('['));
    assert_eq!(rows[0].chars().nth(1), Some('a'));
    assert_eq!(rows[0].chars().nth(2), Some('b'));
    assert_eq!(rows[0].chars().nth(3), Some(']'), "close at width-1");
}

#[test]
fn input_field_key_classifies_editing_keys() {
    // A focused `<input>` owns the editing / caret keys — even when they are
    // boundary no-ops (Home at the start) — but not page navigation (↓ / ↑ /
    // PageDown / PageUp), which must fall through to scrolling while the
    // field stays focused.
    assert!(is_input_field_key(KeyCode::Char('a')));
    assert!(is_input_field_key(KeyCode::Backspace));
    assert!(is_input_field_key(KeyCode::Delete));
    assert!(is_input_field_key(KeyCode::Left));
    assert!(is_input_field_key(KeyCode::Right));
    assert!(is_input_field_key(KeyCode::Home));
    assert!(is_input_field_key(KeyCode::End));
    assert!(!is_input_field_key(KeyCode::Down));
    assert!(!is_input_field_key(KeyCode::Up));
    assert!(!is_input_field_key(KeyCode::PageDown));
    assert!(!is_input_field_key(KeyCode::PageUp));
    assert!(!is_input_field_key(KeyCode::Enter));
    assert!(!is_input_field_key(KeyCode::F(1)));
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
    tl.draw(&arena, root_id, &term_map);

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

#[test]
fn word_boundaries_match_browser_semantics() {
    // `Ctrl`/`Alt`+←/→ word navigation: boundaries fall between word runs and
    // non-word / whitespace runs (browser / vim `b` / `w` semantics).
    let s = "hello brave new world";
    assert_eq!(prev_word_boundary(s, 11), 6, "back to the previous word start");
    assert_eq!(next_word_boundary(s, 0), 5, "forward to the next word end");
    assert_eq!(
        next_word_boundary(s, 5),
        11,
        "whitespace + the next word are covered in one step (browser semantics)"
    );
    assert_eq!(prev_word_boundary(s, 6), 0, "back over a leading word");

    // Punctuation counts as a non-word run (a single boundary hop).
    assert_eq!(prev_word_boundary("a,b", 2), 1, "stop before the comma");
    assert_eq!(next_word_boundary("a,b", 0), 1, "stop after `a`");

    // Clamping at the string ends.
    assert_eq!(prev_word_boundary("ab", 0), 0);
    assert_eq!(next_word_boundary("ab", 2), 2);
    assert_eq!(prev_word_boundary("", 0), 0);
    assert_eq!(next_word_boundary("", 0), 0);
}

#[test]
fn apply_word_key_moves_caret_by_word() {
    let mut value = String::from("hello brave new world");
    let mut cursor = 0;
    let mut anchor = None;

    // Ctrl+Right jumps to the end of the next word.
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Right,
        KeyModifiers::CONTROL,
    ));
    assert_eq!(cursor, 5);
    // Ctrl+Left returns to the previous word start.
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Left,
        KeyModifiers::CONTROL,
    ));
    assert_eq!(cursor, 0);
    // Alt+Left is accepted as a word move too (mac muscle memory).
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Right,
        KeyModifiers::ALT,
    ));
    assert_eq!(cursor, 5);
    // Boundary no-ops report `false` (caret already at the start).
    cursor = 0;
    assert!(!apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Left,
        KeyModifiers::ALT,
    ));
    assert_eq!(cursor, 0);
}

#[test]
fn apply_word_key_deletes_by_word() {
    // Ctrl+Backspace removes the word before the caret.
    let mut value = String::from("hello world");
    let mut cursor = 11;
    let mut anchor = None;
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Backspace,
        KeyModifiers::CONTROL,
    ));
    assert_eq!(value, "hello ");
    assert_eq!(cursor, 6, "caret lands on the deleted word's start");

    // Ctrl+Delete removes the word after the caret.
    let mut value = String::from("hello world");
    let mut cursor = 6;
    let mut anchor = None;
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Delete,
        KeyModifiers::CONTROL,
    ));
    assert_eq!(value, "hello ");
    assert_eq!(cursor, 6);

    // Ctrl+Home / Ctrl+End jump to the line edges (browser caret movement).
    let mut value = String::from("abc def");
    let mut cursor = 3;
    let mut anchor = None;
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Home,
        KeyModifiers::CONTROL,
    ));
    assert_eq!(cursor, 0);
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::End,
        KeyModifiers::CONTROL,
    ));
    assert_eq!(cursor, 7);
}

#[test]
fn apply_word_key_shift_extends_the_selection() {
    // Shift+Ctrl+Right selects from the original caret to the next word end.
    let mut value = String::from("hello brave");
    let mut cursor = 0;
    let mut anchor = None;
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Right,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(cursor, 5);
    assert_eq!(anchor, Some(0), "selection anchor stays at the start");

    // A second word move extends the selection further: the whitespace run
    // and the next word are covered in one step (browser semantics).
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Right,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(cursor, 11);
    assert_eq!(anchor, Some(0));

    // A plain word move (no Shift) drops the selection.
    assert!(apply_input_word_key(
        &mut value,
        &mut cursor,
        &mut anchor,
        KeyCode::Left,
        KeyModifiers::CONTROL,
    ));
    assert_eq!(anchor, None, "plain navigation clears the selection");
}

#[test]
fn collect_input_ids_returns_document_order() {
    init();
    let (root_id, arena) = setup(
        r#"<div><input id="first"><p><input id="second"><input id="third"></p></div>"#,
    );
    let ids = collect_input_ids(&arena);
    let labels: Vec<&str> = ids
        .iter()
        .map(|&id| arena[id].attributes.get("id").map(String::as_str).unwrap_or("?"))
        .collect();
    assert_eq!(labels, vec!["first", "second", "third"], "Tab order follows DOM order");
    assert!(ids.iter().all(|&id| id != root_id), "root is not an input");
}

#[test]
fn key_event_info_maps_every_key_to_browser_fields() {
    // The crossterm → `KeyEventInfo` conversion keeps the browser vocabulary:
    // pages branch on the same `key` / `code` / `keyCode` strings as in a real
    // browser.
    let mut info = key_event_info(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!(info.key, "a");
    assert_eq!(info.code, "KeyA");
    assert_eq!(info.key_code, 'a' as u32);
    assert!(!info.ctrl && !info.shift && !info.alt && !info.meta && !info.repeat);

    info = key_event_info(&KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(info.key, " ");
    assert_eq!(info.code, "Space");
    assert_eq!(info.key_code, 32);

    info = key_event_info(&KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
    assert_eq!(info.code, "Digit5");

    info = key_event_info(&KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(info.key, "ArrowUp");
    assert_eq!(info.key_code, 38);
    assert!(info.shift);

    info = key_event_info(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!((info.key.as_str(), info.key_code), ("Enter", 13));

    info = key_event_info(&KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    assert_eq!((info.key.as_str(), info.code.as_str(), info.key_code), ("F5", "F5", 116));

    info = key_event_info(&KeyEvent::new(KeyCode::Esc, KeyModifiers::CONTROL));
    assert_eq!(info.key, "Escape");
    assert!(info.ctrl);

    // A repeated press is flagged for JS (`event.repeat`).
    let ev = KeyEvent {
        code: KeyCode::Char('x'),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Repeat,
        state: KeyEventState::empty(),
    };
    info = key_event_info(&ev);
    assert!(info.repeat, "repeat presses must be visible to JS listeners");
}

#[test]
fn input_state_clamps_scroll_to_the_content_ceiling() {
    // Regression: growing the terminal (or reloading a shorter document) drops
    // `max_scroll` below the current scroll position. The input loop must
    // follow the render thread's clamp or every wheel / arrow `Rerender` is
    // clamped to the same value in the render thread and dropped as a no-op —
    // the scroll wheel appears dead after a resize.
    let mut state = InputState::new();
    state.scroll_y = 60; // scrolled to the bottom of a 100-row doc @ 24 rows
    state.clamp_scroll_to(76); // ceiling above the position: no clamp
    assert_eq!(state.scroll_y, 60);
    state.clamp_scroll_to(60); // terminal grew to 40 rows: ceiling at 60
    assert_eq!(state.scroll_y, 60);
    state.clamp_scroll_to(59); // grew further / shorter reload: ceiling drops
    assert_eq!(state.scroll_y, 59);

    // The u32::MAX startup placeholder must never clamp a fresh session.
    let mut fresh = InputState::new();
    fresh.scroll_y = 5;
    fresh.clamp_scroll_to(u32::MAX);
    assert_eq!(fresh.scroll_y, 5);
}

// ============================================================
// `<input type=button|submit|reset>` — push-button inputs
// ============================================================

#[test]
fn input_button_types_render_as_bracketed_buttons() {
    init();
    // `button` / `submit` / `reset` paint the `[ label ]` frame instead of a
    // text field; the label comes from the `value` attribute. Each inline
    // replaced element claims its own row (the engine's flow semantics).
    let (rows, _) = render_rows(
        r#"<div><input type="button" value="X"><input type="submit" value="Save"><input type="reset" value="Clear"></div>"#,
    );
    assert!(rows[0].contains("[ X ]"), "button input paints `[ label ]`; row={:?}", rows[0]);
    assert!(rows[1].contains("[ Save ]"), "submit input paints its value label; row={:?}", rows[1]);
    assert!(rows[2].contains("[ Clear ]"), "reset input paints its value label; row={:?}", rows[2]);
}

#[test]
fn input_button_default_labels_submit_reset() {
    init();
    // No `value` attribute → the browser default labels `Submit` / `Reset`.
    let (rows, _) = render_rows(r#"<div><input type="submit"><input type="reset"></div>"#);
    assert!(rows[0].contains("[ Submit ]"), "default submit label; row={:?}", rows[0]);
    assert!(rows[1].contains("[ Reset ]"), "default reset label; row={:?}", rows[1]);
}

#[test]
fn input_button_geometry_sizes_from_label() {
    init();
    // The button input width adapts to its label (`[ text ]`), like `<button>`
    // — not the fixed `width` attribute used for text fields.
    let (root_id, arena) = setup(r#"<div><input id="sub" type="submit" value="Go"><input id="txt" type="text" width="20"></div>"#);
    let boxes = layout_boxes(&arena, root_id);
    let id_of = |id: &str| {
        arena
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
            .map(|(nid, _)| nid)
            .expect(id)
    };
    let submit = find_box(&boxes, id_of("sub")).expect("submit box");
    assert_eq!(submit.rect.width, 6, "`[ Go ]` is 6 columns");
    let text = find_box(&boxes, id_of("txt")).expect("text box");
    assert_eq!(text.rect.width, 20, "text field keeps the width attribute");
}

#[test]
fn input_button_skipped_by_tab_focus_cycle() {
    init();
    // Button inputs activate on click only — they are not text fields, so
    // `Tab` focus cycling must skip them.
    let (_, arena) = setup(
        r#"<div><input id="a" type="text"><input type="submit"><input type="button" value="X"><input id="b" type="text"></div>"#,
    );
    let ids = collect_input_ids(&arena);
    let labels: Vec<&str> = ids
        .iter()
        .map(|&id| arena[id].attributes.get("id").map(String::as_str).unwrap_or("?"))
        .collect();
    assert_eq!(
        labels,
        vec!["a", "b"],
        "button inputs must not enter the text-field tab order"
    );
}

// ============================================================
// placeholder / disabled / number
// ============================================================

#[test]
fn input_placeholder_shows_dimmed_hint_when_empty() {
    init();
    // An empty field with a `placeholder` attribute shows the hint text in
    // grey (the field is unfocused, so no value/caret either).
    let (_, tl) = render_rows(r#"<div><input placeholder="Type here" width="20"></div>"#);
    let hint_cells: Vec<&Block> = tl
        .layout
        .iter()
        .filter(|b| !b.word.is_empty() && b.word != "[" && b.word != "]")
        .collect();
    let text: String = hint_cells.iter().map(|b| b.word.as_str()).collect();
    let text = text.trim();
    assert_eq!(text, "Type here", "placeholder must render in the empty field");
    // Every non-space placeholder glyph (the frame and blank padding cells
    // keep their own colours) must be dimmed grey.
    assert!(
        hint_cells
            .iter()
            .filter(|b| b.word != " ")
            .all(|b| b.fcolor == Some(crossterm::style::Color::DarkGrey)),
        "placeholder hint must be dimmed grey"
    );
}

#[test]
fn input_placeholder_hidden_when_value_present() {
    init();
    // A non-empty `value` wins over the placeholder hint.
    let (_, tl) = render_rows(r#"<div><input placeholder="hint" value="val"></div>"#);
    let text: String = tl
        .layout
        .iter()
        .filter(|b| !b.word.is_empty() && b.word != " " && b.word != "[" && b.word != "]")
        .map(|b| b.word.as_str())
        .collect();
    assert_eq!(text, "val", "the value renders, not the placeholder");
}

#[test]
fn input_disabled_field_is_dimmed() {
    init();
    // A `disabled` text field dims its frame and value (grey) like a disabled
    // `<button>`.
    let (_, tl) = render_rows(r#"<div><input disabled value="off"></div>"#);
    let cells: Vec<&Block> = tl
        .layout
        .iter()
        .filter(|b| !b.word.is_empty() && b.word != " ")
        .collect();
    let text: String = cells.iter().map(|b| b.word.as_str()).collect();
    assert_eq!(text, "[off]", "disabled field keeps its value");
    assert!(
        cells.iter().all(|b| b.fcolor == Some(crossterm::style::Color::DarkGrey)),
        "disabled field must be dimmed grey"
    );
}

#[test]
fn number_input_character_filter_matches_browser() {
    init();
    // The number-editor character set: digits plus sign / decimal / exponent
    // markers. Letters and other symbols are rejected.
    for ok in ['0', '9', '-', '+', '.', 'e', 'E'] {
        assert!(number_input_allows_char(ok), "{ok:?} must be accepted");
    }
    for bad in ['a', 'x', '_', ' ', ',', '#', '你'] {
        assert!(!number_input_allows_char(bad), "{bad:?} must be rejected");
    }
}

#[test]
fn input_checkbox_radio_disabled_is_dimmed() {
    init();
    // A disabled toggle paints its glyph in grey (clicks are already
    // swallowed); the enabled one keeps its normal colour.
    let (_, tl) = render_rows(
        r#"<div><input id="c" type="checkbox" disabled><input id="r" type="radio" disabled checked></div>"#,
    );
    let dark = crossterm::style::Color::DarkGrey;
    let glyphs: Vec<&Block> = tl
        .layout
        .iter()
        .filter(|b| matches!(b.word.as_str(), "☐" | "☑" | "○" | "◉"))
        .collect();
    assert_eq!(glyphs.len(), 2, "both disabled toggles paint their glyph");
    assert!(
        glyphs.iter().all(|b| b.fcolor == Some(dark)),
        "disabled toggles must be dimmed grey"
    );
}

#[test]
fn input_image_renders_as_button() {
    init();
    // `image` is a button-shaped control: it paints the `[ label ]` frame
    // (the browser's own look) even though the terminal has no image — the
    // label comes from its `value` / `alt` with a default fallback.
    let (rows, _) = render_rows(
        r#"<div><input type="image" value="Go"><input type="image" alt="Logo"><input type="image"></div>"#,
    );
    assert!(rows[0].contains("[ Go ]"), "image input uses its value label; row={:?}", rows[0]);
    assert!(rows[1].contains("[ Logo ]"), "image input falls back to alt; row={:?}", rows[1]);
    assert!(
        rows[2].contains("[ Submit Query ]"),
        "image input defaults to `Submit Query`; row={:?}",
        rows[2]
    );
}

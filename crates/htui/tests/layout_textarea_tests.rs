//! `<textarea>` tests — multi-line editable field.
//!
//! The box is sized from `cols` / `rows`, renders the value (its text
//! content) inside a framed rectangle, and supports multi-line editing
//! (Enter inserts a newline, ↑/↓ move the caret between visual lines, the
//! caret-driven scroll keeps the caret visible). Blur commits the edited
//! text back into the textarea's child text nodes.

use compact_str::CompactString;
use crossterm::event::KeyCode;
use crossterm::style::Color;
use htui::events::{EventBindings, Geometry, HitMap, interaction::HoverState};
use htui::html::css_parse::DynamicState;
use htui::html::html_parse;
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, apply_message};
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::{InputRenderState, RenderCmd, TermLayout, apply_textarea_key};

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

fn setup(html: &str) -> (NodeId, NodeArena) {
    let mut parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = parser.parse();
    if !embedded.trim().is_empty() {
        let mut css_parser = htui::html::css_parse::Parser::new(&embedded);
        let rules = css_parser.parse();
        htui::html::css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    (root_id, arena)
}

fn layout_boxes(html: &str) -> (Vec<htui::layout::LayoutBox>, NodeArena) {
    let (root_id, arena) = setup(html);
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

fn find_node_by_id(arena: &NodeArena, id: &str) -> NodeId {
    arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(nid, _)| nid)
        .expect("element with id exists")
}

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

fn build_geometry(html: &str) -> (ScriptEngine, Geometry) {
    let engine = ScriptEngine::new(html, "", EventBindings::default()).expect("engine initializes");
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

/// Evaluate `String(expr)` against the engine and return its text.
fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

#[test]
fn test_textarea_renders_default_value() {
    // `cols`/`rows` size the box (width = cols, height = rows); the initial
    // value is the element's text content.
    let (boxes, arena) = layout_boxes(r#"<textarea id="t" cols="6" rows="2">ab&#10;cd</textarea>"#);
    let t = find_node_by_id(&arena, "t");
    let b = find_box(&boxes, t).expect("textarea box");
    assert_eq!(b.rect.width, 6, "6 cols");
    assert_eq!(b.rect.height, 2, "2 content rows");

    let term_map = mock_term_map();
    let (root_id, arena) = setup(r#"<textarea id="t" cols="6" rows="2">ab&#10;cd</textarea>"#);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let chars: Vec<&str> = tl
        .layout
        .iter()
        .filter(|b| b.word != " ")
        .map(|b| b.word.as_str())
        .collect();
    let joined = chars.join("");
    assert!(joined.contains("ab"), "first line paints (got {joined:?})");
    assert!(joined.contains("cd"), "second line paints (got {joined:?})");
}

#[test]
fn test_textarea_default_size() {
    // No `cols`/`rows`: browser defaults 20 × 2.
    let (boxes, arena) = layout_boxes(r#"<textarea>hi</textarea>"#);
    let t = arena
        .iter()
        .find(|(_, n)| matches!(n.node_type, htui::tdom::node::NodeType::Textarea))
        .map(|(id, _)| id)
        .expect("textarea node");
    let b = find_box(&boxes, t).expect("textarea box");
    assert_eq!(b.rect.width, 20, "20 default cols");
    assert_eq!(b.rect.height, 2, "2 default rows");
}

#[test]
fn test_textarea_key_enter_inserts_newline() {
    let mut value = CompactString::from("ab");
    let mut cursor = 2;
    assert!(apply_textarea_key(
        &mut value,
        &mut cursor,
        10,
        KeyCode::Enter
    ));
    assert_eq!(value, "ab\n");
    assert_eq!(cursor, 3);
    // A literal newline char inserts the same way.
    let mut value = CompactString::from("a");
    let mut cursor = 1;
    assert!(apply_textarea_key(
        &mut value,
        &mut cursor,
        10,
        KeyCode::Char('\n')
    ));
    assert_eq!(value, "a\n");
}

#[test]
fn test_textarea_key_up_down_moves_caret_between_lines() {
    // Visual lines are "abc" / "def"; moving down from (0,1) lands on (1,1).
    let mut value = CompactString::from("abc\ndef");
    let mut cursor = 1;
    assert!(
        apply_textarea_key(&mut value, &mut cursor, 10, KeyCode::Down),
        "down should move to the next line"
    );
    assert_eq!(cursor, 5, "char 4 ('d') + column 1");
    assert!(
        apply_textarea_key(&mut value, &mut cursor, 10, KeyCode::Up),
        "up should move back"
    );
    assert_eq!(cursor, 1);
    // Boundaries are no-ops.
    assert!(!apply_textarea_key(
        &mut value,
        &mut cursor,
        10,
        KeyCode::Up
    ));
    cursor = 7;
    assert!(!apply_textarea_key(
        &mut value,
        &mut cursor,
        10,
        KeyCode::Down
    ));
}

#[test]
fn test_textarea_wrap_moves_caret_across_wrapped_lines() {
    // content_width 3: "abcdef" wraps to "abc" / "def" — down from (0,1)
    // lands on the second visual line's column 1.
    let mut value = CompactString::from("abcdef");
    let mut cursor = 1;
    assert!(apply_textarea_key(
        &mut value,
        &mut cursor,
        3,
        KeyCode::Down
    ));
    assert_eq!(
        cursor, 4,
        "visual row 1 starts at char 3, column 1 -> char 4"
    );
}

#[test]
fn test_textarea_blur_commits_text_content() {
    let (mut engine, geo) = build_geometry(r#"<textarea id="t">old</textarea>"#);
    let map = HitMap::build(&geo);
    let t = find_node_by_id(&engine.arena(), "t");
    let cmds = apply(
        &mut engine,
        &map,
        ScriptMsg::BlurInput {
            node_id: t,
            value: "new\nvalue".into(),
        },
    );
    let arena = engine.arena();
    assert_eq!(
        arena.get(t).unwrap().text_content(&arena),
        "new\nvalue",
        "textarea blur must commit into the text content"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, RenderCmd::RenderArena { .. })),
        "commit must mark the node dirty"
    );
}

#[test]
fn test_textarea_blur_same_value_no_change_event() {
    // Committing an unchanged textarea value is a no-op (no dirty marker).
    let (mut engine, geo) = build_geometry(r#"<textarea id="t">same</textarea>"#);
    let map = HitMap::build(&geo);
    let t = find_node_by_id(&engine.arena(), "t");
    let cmds = apply(
        &mut engine,
        &map,
        ScriptMsg::BlurInput {
            node_id: t,
            value: "same".into(),
        },
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, RenderCmd::RenderArena { .. })),
        "unchanged value must not re-render"
    );
}

#[test]
fn test_textarea_scroll_keeps_caret_visible() {
    // rows=1 with four lines: a focused caret on the last line scrolls the
    // box so only that line is visible.
    let term_map = mock_term_map();
    let (root_id, arena) =
        setup(r#"<textarea id="t" cols="6" rows="1">a&#10;b&#10;c&#10;d</textarea>"#);
    let t = find_node_by_id(&arena, "t");
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.input_states.insert(
        t,
        InputRenderState {
            value: "a\nb\nc\nd".into(),
            cursor: 7,
            focused: true,
            selection: None,
        },
    );
    tl.draw(&arena, root_id, &term_map);
    assert!(
        tl.layout.iter().any(|b| b.word == "d"),
        "caret line must be visible"
    );
    assert!(
        !tl.layout.iter().any(|b| b.word == "a"),
        "scrolled-out lines must not paint"
    );
}

#[test]
fn test_textarea_value_getter_reads_text_content() {
    // A `<textarea>`'s DOM value is its text content, not a `value`
    // attribute — `el.value` must reflect the text nodes.
    let mut engine = ScriptEngine::new(
        r#"<textarea id="t">first&#10;line</textarea>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('t').value"),
        "first\nline",
        "el.value must read the text content"
    );
    // textContent agrees (same underlying text nodes).
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('t').textContent"),
        "first\nline"
    );
}

#[test]
fn test_textarea_value_setter_writes_text_content() {
    let mut engine = ScriptEngine::new(
        r#"<textarea id="t">old</textarea>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "(function(){ document.getElementById('t').value = 'new\\nvalue'; })()",
    );
    let arena = engine.arena();
    let t = find_node_by_id(&arena, "t");
    assert_eq!(
        arena.get(t).unwrap().text_content(&arena),
        "new\nvalue",
        "el.value setter must write the text content"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('t').value"),
        "new\nvalue"
    );
    // The dirty marker routes a re-render through the normal path.
    let dirty = engine.take_dirty();
    assert!(
        dirty.nodes.contains(&t),
        "value write must mark the node dirty: {dirty:?}"
    );
}

#[test]
fn test_textarea_value_setter_short_circuits_unchanged() {
    // Setting the same value again must not schedule a re-render.
    let mut engine = ScriptEngine::new(
        r#"<textarea id="t">same</textarea>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "(function(){ document.getElementById('t').value = 'same'; })()",
    );
    let dirty = engine.take_dirty();
    assert!(
        dirty.is_empty(),
        "unchanged textarea value must not re-render: {dirty:?}"
    );
}

#[test]
fn test_textarea_value_setter_clears_to_empty() {
    let mut engine = ScriptEngine::new(
        r#"<textarea id="t">stuff</textarea>"#,
        "",
        EventBindings::default(),
    )
    .expect("engine initializes");
    eval_str(
        &mut engine,
        "(function(){ document.getElementById('t').value = ''; })()",
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('t').value"),
        "",
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('t').textContent"),
        "",
    );
}

#[test]
fn test_textarea_focused_paints_inverted_caret() {
    // The focused caret cell is inverted (fg/bg swapped) at the cursor.
    let term_map = mock_term_map();
    let (root_id, arena) = setup(r#"<textarea id="t" cols="6" rows="1">ab</textarea>"#);
    let t = find_node_by_id(&arena, "t");
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.input_states.insert(
        t,
        InputRenderState {
            value: "ab".into(),
            cursor: 1,
            focused: true,
            selection: None,
        },
    );
    tl.draw(&arena, root_id, &term_map);
    let caret_idx = tl
        .layout
        .iter()
        .position(|b| b.fcolor == Some(Color::DarkGrey) && b.bcolor == Some(Color::White))
        .expect("inverted caret cell");
    let cols = term_map.columns as usize;
    assert_eq!(caret_idx % cols, 1, "caret sits at column 1 of the content");
}

#[test]
fn test_textarea_caret_at_line_end_stays_on_its_line() {
    // cursor=2 sits between 'b' and '\n' in "ab\ncd": the caret must paint at
    // the end of the first line (after 'b'), NOT at the start of the second
    // line. Otherwise Backspace / Delete remove characters that are not the
    // ones either side of the painted caret (the edit buffer is indexed by
    // character, so a line-end cursor must display on the line it ends).
    let term_map = mock_term_map();
    let (root_id, arena) = setup(r#"<textarea id="t" cols="8" rows="3">ab&#10;cd</textarea>"#);
    let t = find_node_by_id(&arena, "t");
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.input_states.insert(
        t,
        InputRenderState {
            value: "ab\ncd".into(),
            cursor: 2,
            focused: true,
            selection: None,
        },
    );
    tl.draw(&arena, root_id, &term_map);
    let caret_idx = tl
        .layout
        .iter()
        .position(|b| b.fcolor == Some(Color::DarkGrey) && b.bcolor == Some(Color::White))
        .expect("inverted caret cell");
    let cols = term_map.columns as usize;
    let (row, col) = (caret_idx / cols, caret_idx % cols);
    // The textarea box sits at y=0 and the content starts at (0,0) (no
    // frame); the caret paints at column 2 — after 'ab' (x=0..1 'ab', x=2
    // caret), not on the second content row.
    assert_eq!(
        (row, col),
        (0, 2),
        "caret paints after 'ab' on the first content line"
    );
}

#[test]
fn test_textarea_backspace_delete_match_caret_position() {
    // "ab\ncd" with the caret after 'b' (cursor 2): Backspace removes 'b'
    // and Delete removes the '\n' — exactly the characters either side of
    // the painted caret (see the caret-position test above).
    let mut value = CompactString::from("ab\ncd");
    let mut cursor = 2;
    assert!(apply_textarea_key(
        &mut value,
        &mut cursor,
        10,
        KeyCode::Backspace
    ));
    assert_eq!(
        value, "a\ncd",
        "Backspace removes the char before the caret"
    );
    assert_eq!(cursor, 1);

    let mut value = CompactString::from("ab\ncd");
    let mut cursor = 2;
    assert!(apply_textarea_key(
        &mut value,
        &mut cursor,
        10,
        KeyCode::Delete
    ));
    assert_eq!(value, "abcd", "Delete at a line end removes the newline");
    assert_eq!(cursor, 2);
}

#[test]
fn test_textarea_wide_char_caret_matches_editing() {
    // A CJK character occupies two cells: the caret after '中' paints at
    // column 2 and Backspace removes the whole character (never half of a
    // UTF-8 sequence / a single cell).
    let mut value = CompactString::from("中文");
    let mut cursor = 1;
    assert!(apply_textarea_key(
        &mut value,
        &mut cursor,
        10,
        KeyCode::Backspace
    ));
    assert_eq!(value, "文", "Backspace removes the full wide character");
    assert_eq!(cursor, 0);

    let term_map = mock_term_map();
    let (root_id, arena) = setup(r#"<textarea id="t" cols="6" rows="1">中文</textarea>"#);
    let t = find_node_by_id(&arena, "t");
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.input_states.insert(
        t,
        InputRenderState {
            value: "中文".into(),
            cursor: 1,
            focused: true,
            selection: None,
        },
    );
    tl.draw(&arena, root_id, &term_map);
    let caret_idx = tl
        .layout
        .iter()
        .position(|b| b.fcolor == Some(Color::DarkGrey) && b.bcolor == Some(Color::White))
        .expect("inverted caret cell");
    let cols = term_map.columns as usize;
    assert_eq!(
        caret_idx % cols,
        2,
        "caret paints after the two-cell '中' (x=0..1 '中', x=2 caret)"
    );
}

/// A `<textarea>` whose rect exceeds the terminal must paint a clamped,
/// intact background instead of wrapping its frame writes into the next row.
///
/// As a flex item with `flex-shrink: 0` the field keeps its full CSS width
/// even when the viewport is narrower; the painter clamps the rect to the
/// canvas (like `paint_block` does) so the field's background reads cleanly
/// and the content below is not corrupted.
#[test]
fn wide_textarea_clamps_background_to_canvas() {
    let html = r#"
      <style>
        .row { display: flex; }
        textarea { width: 40ch; height: 3ch; flex-shrink: 0; }
      </style>
      <div class="row"><textarea>content</textarea></div>
      <p>below line</p>
    "#;
    let (root_id, arena) = setup(html);
    let narrow = TermMap {
        columns: 20,
        rows: 10,
    };
    let mut tl = TermLayout::new(&narrow, 1.0);
    tl.draw(&arena, root_id, &narrow);

    let cols = narrow.columns as usize;
    // The first row carries the field's default control background across
    // the clamped width (the field content starts at the left edge).
    assert_eq!(
        tl.layout[0].bcolor,
        Some(crossterm::style::Color::DarkGrey),
        "textarea background fills the clamped row"
    );
    // Content rows keep the fill too.
    let mid = &tl.layout[cols];
    assert_eq!(
        mid.bcolor,
        Some(crossterm::style::Color::DarkGrey),
        "content row background"
    );
    // The following paragraph starts on its own clean row (no wrapped frame
    // writes landing on it).
    let rows_txt: Vec<String> = tl
        .layout
        .chunks(cols)
        .enumerate()
        .map(|(r, row)| {
            format!(
                "{r:02}|{}",
                row.iter()
                    .map(|c| c.word.as_str())
                    .collect::<String>()
                    .trim_end()
            )
        })
        .collect();
    for s in &rows_txt {
        if !s.ends_with('|') {
            eprintln!("{s}");
        }
    }
    let below: Vec<(usize, String)> = tl
        .layout
        .chunks(cols)
        .enumerate()
        .map(|(r, row)| (r, row.iter().map(|c| c.word.as_str()).collect::<String>()))
        .filter(|(_, s)| !s.trim().is_empty())
        .collect();
    assert!(
        below
            .iter()
            .any(|(_, s)| s.trim_end().starts_with("below line")),
        "content below the wide field must stay uncorrupted, got {:?}",
        below
    );
}

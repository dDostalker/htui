//! `<output>` / `<optgroup>` / `<datalist>` / `<fieldset>` / `<legend>` /
//! `<progress>` / `<meter>` tests — the remaining form elements.
//!
//! - `<output>` renders its text inline; `value` reads / replaces the text.
//! - `<optgroup>` adds non-selectable group headers to a `<select>` dropdown;
//!   navigation and clicks skip them.
//! - `<datalist>` never renders but feeds the autocomplete dropdown of an
//!   `<input list>` (opened on focus, filtered by the field text).
//! - `<fieldset>` is a block with the UA default border + margins; `<legend>`
//!   is a plain block inside it.
//! - `<progress>` / `<meter>` render a `█`/`░` bar from `value` / `max`
//!   (`<meter>` colours it by `low` / `high` / `optimum`).

use crossterm::event::MouseButton;
use crossterm::style::Color;
use htui::events::{EventBindings, Geometry, HitMap, HoverState};
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::layoutbox::geometry::{SelectOption, SelectRow};
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, apply_message};
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{
    InputState, OverlayKind, RenderCmd, TermLayout, handle_click, handle_mouse_down,
};

const COLS: u16 = 80;
const ROWS: u16 = 24;

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

fn find_node_by_type(arena: &NodeArena, target: &NodeType) -> Option<NodeId> {
    arena
        .iter()
        .find(|(_, n)| n.node_type == *target)
        .map(|(id, _)| id)
}

/// Count canvas cells whose glyph equals `word`.
fn count_word(tl: &TermLayout, word: &str) -> usize {
    tl.layout.iter().filter(|b| b.word == word).count()
}

fn eval_str(engine: &mut ScriptEngine, expr: &str) -> String {
    let v = engine
        .eval_source(&format!("String({expr})"))
        .expect("eval must succeed");
    v.as_string()
        .map(|s| s.to_std_string_lossy())
        .unwrap_or_default()
}

/// Build the engine + geometry for a document (mirrors the runtime path).
fn build_geometry(html: &str) -> (ScriptEngine, Geometry) {
    let engine = ScriptEngine::new(html, EventBindings::default()).expect("engine initializes");
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
        arena,
        boxes,
        root_id,
    };
    (engine, geo)
}

/// Mouse-down at canvas coordinates, exactly like the input loop (the
/// dropdown open state is set synchronously; queued messages are left to the
/// caller when needed).
fn mouse_down(
    state: &mut InputState,
    engine: &ScriptEngine,
    map: &HitMap,
    x: u32,
    y: u32,
    button: MouseButton,
) -> (Vec<ScriptMsg>, Vec<RenderCmd>) {
    let arena = engine.arena();
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_down(state, map, &arena, x, y, button, &mut msgs, &mut cmds);
    (msgs, cmds)
}

/// A canvas point inside `rect` (its centre).
fn center(rect: htui::layout::Rect) -> (u32, u32) {
    (
        rect.x as u32 + rect.width.max(1) as u32 / 2,
        rect.y as u32 + rect.height.max(1) as u32 / 2,
    )
}

// ============================================================
// <output>
// ============================================================

#[test]
fn test_output_renders_inline_text() {
    init();
    let term_map = mock_term_map();
    let tl = draw("<p>Result: <output id=\"o\">42</output></p>", "");
    let cols = term_map.columns as usize;
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "4")
        .expect("output text renders");
    assert_eq!(idx % cols, 8, "output flows inline right after the label");
    assert_eq!(
        idx / cols, 1,
        "output shares the paragraph's text row (UA margin-top=1)"
    );
}

#[test]
fn test_output_value_property_reads_and_writes_text() {
    init();
    let mut engine = ScriptEngine::new(r#"<output id="o">42</output>"#, EventBindings::default())
        .expect("engine");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('o').value"),
        "42",
        "output.value reads the text content"
    );
    let _ = engine.eval_source("document.getElementById('o').value = '50'");
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('o').textContent"),
        "50",
        "setting output.value replaces the text content"
    );
}

// ============================================================
// <fieldset> / <legend>
// ============================================================

#[test]
fn test_fieldset_renders_border_and_margins() {
    init();
    let (boxes, arena) = layout_boxes(
        "<fieldset><legend>Group</legend><p>content</p></fieldset>",
        "",
    );
    let fs = find_box_by_type(&boxes, &NodeType::Fieldset, &arena).expect("fieldset box");
    assert_eq!(fs.rect.x, 0);
    assert_eq!(fs.rect.y, 1, "UA top margin reserves one row");
    assert!(fs.rect.height >= 4, "border + legend + content + border");
    let legend = find_box_by_type(&boxes, &NodeType::Legend, &arena).expect("legend box");
    assert_eq!(legend.rect.height, 1);
    let tl = draw(
        "<fieldset><legend>Group</legend><p>content</p></fieldset>",
        "",
    );
    assert!(count_word(&tl, "┌") > 0, "UA border frame draws");
    assert!(
        count_word(&tl, "G") > 0,
        "legend text renders inside the frame"
    );
    assert!(count_word(&tl, "c") > 0, "fieldset content renders");
}

#[test]
fn test_legend_alone_renders_as_block() {
    init();
    let (boxes, arena) = layout_boxes("<legend>Caption</legend>", "");
    let legend = find_box_by_type(&boxes, &NodeType::Legend, &arena).expect("legend box");
    assert_eq!(legend.rect.y, 0);
    let tl = draw("<legend>Caption</legend>", "");
    assert!(count_word(&tl, "C") > 0);
}

// ============================================================
// <progress> / <meter>
// ============================================================

#[test]
fn test_progress_renders_half_bar() {
    init();
    let tl = draw("<progress value=\"0.5\"></progress>", "");
    let filled = count_word(&tl, "█");
    let empty = count_word(&tl, "░");
    assert!(filled > 0 && empty > 0, "half bar has both glyphs");
    let ratio = filled as f64 / (filled + empty) as f64;
    assert!((ratio - 0.5).abs() < 0.05, "fill ratio tracks value/max");
}

#[test]
fn test_progress_full_and_empty() {
    init();
    let tl = draw("<progress value=\"1\"></progress>", "");
    assert!(count_word(&tl, "█") > 0);
    assert_eq!(count_word(&tl, "░"), 0, "full bar has no empty cells");
    let tl = draw("<progress value=\"0\"></progress>", "");
    assert_eq!(count_word(&tl, "█"), 0);
    assert!(count_word(&tl, "░") > 0);
}

#[test]
fn test_progress_author_width_controls_bar() {
    init();
    let (boxes, arena) = layout_boxes(
        "<progress value=\"0.5\"></progress>",
        "progress { width: 20ch; }",
    );
    let p = find_box_by_type(&boxes, &NodeType::Progress, &arena).expect("progress box");
    assert_eq!(p.rect.width, 20, "CSS width constrains the bar");
    let tl = draw(
        "<progress value=\"0.5\"></progress>",
        "progress { width: 20ch; }",
    );
    assert_eq!(count_word(&tl, "█"), 10);
    assert_eq!(count_word(&tl, "░"), 10);
}

#[test]
fn test_meter_ratio_from_min_max() {
    init();
    let tl = draw(
        "<meter value=\"25\" min=\"0\" max=\"100\" low=\"20\" high=\"80\" optimum=\"50\"></meter>",
        "",
    );
    let filled = count_word(&tl, "█");
    let empty = count_word(&tl, "░");
    let ratio = filled as f64 / (filled + empty) as f64;
    assert!(
        (ratio - 0.25).abs() < 0.05,
        "meter fills by (value-min)/(max-min)"
    );
}

#[test]
fn test_meter_bar_color_tiers() {
    init();
    // value within [low, high] and at the optimum → green.
    let tl = draw(
        "<meter value=\"0.5\" low=\"0.2\" high=\"0.8\" optimum=\"0.5\"></meter>",
        "",
    );
    let filled: Vec<_> = tl.layout.iter().filter(|b| b.word == "█").collect();
    assert!(!filled.is_empty());
    assert!(
        filled.iter().all(|b| b.fcolor == Some(Color::Green)),
        "good-range meter paints green"
    );
    // value below low → red.
    let tl = draw(
        "<meter value=\"0.1\" low=\"0.4\" high=\"0.9\" optimum=\"0.5\"></meter>",
        "",
    );
    let filled: Vec<_> = tl.layout.iter().filter(|b| b.word == "█").collect();
    assert!(!filled.is_empty());
    assert!(
        filled.iter().all(|b| b.fcolor == Some(Color::Red)),
        "below-low meter paints red"
    );
}

// ============================================================
// <optgroup>
// ============================================================

const SELECT_HTML: &str = r#"<select id="s">
  <optgroup label="Fruits"><option value="a">Apple</option><option value="b">Banana</option></optgroup>
  <optgroup label="Veg"><option value="c">Carrot</option></optgroup>
  <option value="d">Plain</option>
</select>"#;

#[test]
fn test_optgroup_builds_group_header_rows() {
    init();
    let (engine, geo) = build_geometry(SELECT_HTML);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("select region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    let _ = mouse_down(&mut state, &engine, &map, x, y, MouseButton::Left);
    let open = state.open_select.as_ref().expect("select dropdown opens");

    assert_eq!(
        open.options.len(),
        4,
        "optgroup options flatten into the list"
    );
    assert_eq!(open.options[0].group.as_deref(), Some("Fruits"));
    assert_eq!(open.options[1].group.as_deref(), Some("Fruits"));
    assert_eq!(open.options[2].group.as_deref(), Some("Veg"));
    assert_eq!(
        open.options[3].group, None,
        "ungrouped option keeps no group"
    );
    // rows: Group(Fruits), Item0, Item1, Group(Veg), Item2, Item3
    assert_eq!(
        open.rows.len(),
        6,
        "one header row per optgroup plus the options"
    );
    assert!(matches!(&open.rows[0], SelectRow::Group(g) if g == "Fruits"));
    assert!(matches!(&open.rows[3], SelectRow::Group(g) if g == "Veg"));
    assert!(matches!(&open.rows[1], SelectRow::Item(i) if *i == 0));
    assert!(matches!(&open.rows[5], SelectRow::Item(i) if *i == 3));
}

#[test]
fn test_optgroup_header_click_selects_first_option() {
    init();
    let (engine, geo) = build_geometry(SELECT_HTML);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("select region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    let _ = mouse_down(&mut state, &engine, &map, x, y, MouseButton::Left);
    let open = state.open_select.clone().expect("dropdown opens");

    // Click row 0 — the "Fruits" group header. The header itself has no
    // value, so it commits the group's first option ("Apple"/"a") — every
    // visible row is actionable.
    let (msgs, _) = mouse_down(
        &mut state,
        &engine,
        &map,
        open.rect.x as u32,
        open.rect.y as u32,
        MouseButton::Left,
    );
    assert!(
        state.open_select.is_none(),
        "a header click commits and closes the dropdown"
    );
    assert!(
        msgs.iter()
            .any(|m| matches!(m, ScriptMsg::SelectPick { value, .. } if value == "a")),
        "the group header picks its first option"
    );
}

#[test]
fn test_optgroup_option_click_picks_value() {
    init();
    let (engine, geo) = build_geometry(SELECT_HTML);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("s").expect("select region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    let _ = mouse_down(&mut state, &engine, &map, x, y, MouseButton::Left);
    let open = state.open_select.clone().expect("dropdown opens");

    // Click row 1 — the "Apple" option (row 0 is the "Fruits" header). The
    // row index must map to the *option* index, not the row index: with a
    // header row above, passing the row through would pick "Banana".
    let (msgs, _) = mouse_down(
        &mut state,
        &engine,
        &map,
        open.rect.x as u32,
        open.rect.y.saturating_add(1) as u32,
        MouseButton::Left,
    );
    assert!(
        state.open_select.is_none(),
        "an option click closes the dropdown"
    );
    assert!(
        msgs.iter()
            .any(|m| matches!(m, ScriptMsg::SelectPick { value, .. } if value == "a")),
        "the first option under a group header must commit its own value"
    );

    // Reopen and click row 4 — "Carrot", the option directly under the
    // second header (row 3 = "Veg"): maps to option index 2, not 4.
    let _ = mouse_down(&mut state, &engine, &map, x, y, MouseButton::Left);
    let open = state.open_select.clone().expect("dropdown reopens");
    let (msgs, _) = mouse_down(
        &mut state,
        &engine,
        &map,
        open.rect.x as u32,
        open.rect.y.saturating_add(4) as u32,
        MouseButton::Left,
    );
    assert!(
        msgs.iter()
            .any(|m| matches!(m, ScriptMsg::SelectPick { value, .. } if value == "c")),
        "an option after a header maps to its own option index"
    );
}

// ============================================================
// <datalist>
// ============================================================

#[test]
fn test_datalist_never_renders_but_keeps_options() {
    init();
    let (boxes, arena) = layout_boxes(
        "<div>shown</div><datalist id=\"d\"><option value=\"a\">Apple</option></datalist>",
        "",
    );
    assert!(
        find_box_by_type(&boxes, &NodeType::Datalist, &arena).is_none(),
        "datalist produces no box"
    );
    let d = find_node_by_type(&arena, &NodeType::Datalist).expect("datalist stays in DOM");
    assert_eq!(arena[d].children.len(), 1, "options stay in the DOM");
    assert_eq!(arena[arena[d].children[0]].node_type, NodeType::HtmlOption);
    let tl = draw(
        "<div>shown</div><datalist id=\"d\"><option value=\"a\">Apple</option></datalist>",
        "",
    );
    assert_eq!(count_word(&tl, "A"), 0, "datalist content never paints");
}

#[test]
fn test_datalist_autocomplete_opens_on_focus() {
    init();
    let (engine, geo) = build_geometry(
        r#"<datalist id="d">
           <option value="Apple"></option>
           <option value="Banana"></option>
         </datalist>
         <input id="i" list="d">"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("i").expect("input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    let _ = mouse_down(&mut state, &engine, &map, x, y, MouseButton::Left);
    let open = state
        .open_select
        .as_ref()
        .expect("datalist dropdown opens on focus");
    assert_eq!(open.kind, OverlayKind::Datalist);
    assert_eq!(open.options.len(), 2, "empty field shows every suggestion");
    assert_eq!(open.options[0].value, "Apple");
    assert_eq!(open.options[1].value, "Banana");
}

#[test]
fn test_datalist_filters_by_field_value() {
    init();
    // The field's committed value ("Ba") becomes the filter: only Banana
    // matches the case-insensitive substring.
    let (engine, geo) = build_geometry(
        r#"<datalist id="d">
           <option value="Apple"></option>
           <option value="Banana"></option>
         </datalist>
         <input id="i" list="d" value="Ba">"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("i").expect("input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    let _ = mouse_down(&mut state, &engine, &map, x, y, MouseButton::Left);
    let open = state.open_select.as_ref().expect("dropdown opens");
    assert_eq!(open.options.len(), 1, "filtered to matching suggestions");
    assert_eq!(open.options[0].value, "Banana");
}

#[test]
fn test_datalist_ignored_without_list_attribute() {
    init();
    let (engine, geo) = build_geometry(r#"<input id="i">"#);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("i").expect("input region");
    let (x, y) = center(region.rect);

    let mut state = InputState::new();
    let _ = mouse_down(&mut state, &engine, &map, x, y, MouseButton::Left);
    assert!(
        state.open_select.is_none(),
        "an input without a list attribute opens no dropdown"
    );
}

#[test]
fn visual_test_form_elements_document_initializes() {
    init();
    // Smoke test mirroring `visual_test_details_document_initializes`: the
    // demo page's `<script>` initializes, clicking Compute drives the
    // calculator end-to-end (output.value replaces its text), the grouped
    // `<select>` opens with optgroup header rows, and the `<datalist>` input
    // opens its autocomplete dropdown.
    let html = include_str!("../../../visual_test/src/form_elements/index.html");
    let (mut engine, geo) = build_geometry(html);
    let map = HitMap::build(&geo);
    let mut state = InputState::new();

    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "ready — click Compute / Apply, try the grouped select and the datalist",
        "the demo boots with its initial status line"
    );

    // Compute: 3 + 4 → output.value replaces the text content.
    let region = map
        .resolve_by_id("compute-btn")
        .expect("compute button region");
    let (x, y) = center(region.rect);
    let mut hover = HoverState::new();
    let arena = engine.arena();
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_click(&mut state, &map, &arena, x, y, &mut msgs, &mut cmds);
    for msg in msgs {
        let ev = apply_message(&mut engine, &mut hover, &map, msg);
        cmds.extend(ev.cmds);
    }
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('sum').textContent"),
        "7",
        "the compute handler updates output via its value setter"
    );

    // Click the grouped select: optgroup labels become header rows.
    let region = map.resolve_by_id("fruit").expect("fruit select region");
    let (x, y) = center(region.rect);
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_down(
        &mut state,
        &map,
        &engine.arena(),
        x,
        y,
        MouseButton::Left,
        &mut msgs,
        &mut cmds,
    );
    let open = state.open_select.as_ref().expect("fruit dropdown opens");
    assert_eq!(open.kind, OverlayKind::Select);
    assert!(matches!(&open.rows[0], SelectRow::Group(g) if g == "Citrus"));
    assert!(matches!(&open.rows[2], SelectRow::Item(i) if *i == 1));

    // Click the datalist input: the autocomplete opens with every suggestion.
    // First close the fruit dropdown (its overlay overlaps the input below).
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_down(
        &mut state,
        &map,
        &engine.arena(),
        95,
        39,
        MouseButton::Left,
        &mut msgs,
        &mut cmds,
    );
    assert!(
        state.open_select.is_none(),
        "a blank click closes the dropdown"
    );
    let region = map.resolve_by_id("lang").expect("lang input region");
    let (x, y) = center(region.rect);
    let mut msgs = Vec::new();
    let mut cmds = Vec::new();
    handle_mouse_down(
        &mut state,
        &map,
        &engine.arena(),
        x,
        y,
        MouseButton::Left,
        &mut msgs,
        &mut cmds,
    );
    let open = state.open_select.as_ref().expect("datalist dropdown opens");
    assert_eq!(open.kind, OverlayKind::Datalist);
    assert_eq!(open.options.len(), 4, "all datalist suggestions shown");
}

// ============================================================
// helpers shared with the select row model
// ============================================================

/// Compile-time sanity: the `SelectOption` struct fields used by the tests.
#[test]
fn test_select_option_shape() {
    init();
    let opt = SelectOption {
        value: "v".into(),
        label: "l".into(),
        group: None,
    };
    assert_eq!(opt.value, "v");
    assert_eq!(opt.label, "l");
}

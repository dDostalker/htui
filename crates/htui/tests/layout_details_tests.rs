//! `<details>` / `<summary>` tests — the disclosure widget.
//!
//! A `<details>` element renders as a toggle: while the `open` attribute is
//! absent only its first `<summary>` child is visible (nothing at all when
//! there is no `<summary>`); with `open` present every child renders. The
//! `<summary>` carries a browser-style disclosure marker (`▸` closed / `▾`
//! open), and clicking it natively toggles the `open` attribute and fires a
//! `toggle` event on the `<details>`.

use crossterm::style::Color;
use htui::events::{EventBindings, Geometry, HitMap, HoverState};
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, layout_tree};
use htui::script::{ScriptEngine, ScriptMsg, apply_message};
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

/// Layout the document and return the box tree plus the arena needed to map
/// boxes back to node types.
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

/// Render the document onto the terminal canvas.
fn draw(html: &str) -> (TermMap, TermLayout) {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    (term_map, tl)
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

/// Whether any canvas cell carries exactly `word` as its glyph.
fn canvas_has_word(tl: &TermLayout, word: &str) -> bool {
    tl.layout.iter().any(|b| b.word == word)
}

/// Build the engine + geometry for a document (mirrors the runtime path: the
/// engine owns the live arena and runs scripts at construction).
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

#[test]
fn test_closed_details_renders_only_first_summary() {
    init();
    // No `open` attribute: the browser hides every child but the first
    // `<summary>`. The `<p>` after it must produce no box.
    let (boxes, arena) = layout_boxes("<details><summary>More</summary><p>Secret</p></details>");
    let summary = find_box_by_type(&boxes, &NodeType::Summary, &arena)
        .expect("summary must render its own box");
    assert_eq!(summary.rect.height, 1, "summary is one row");
    assert!(
        find_box_by_type(&boxes, &NodeType::P, &arena).is_none(),
        "hidden content must not produce a box"
    );
    let (_, tl) = draw("<details><summary>More</summary><p>Secret</p></details>");
    assert!(
        canvas_has_word(&tl, "▸"),
        "closed details shows the closed marker"
    );
    assert!(canvas_has_word(&tl, "M"), "summary text renders");
    assert!(!canvas_has_word(&tl, "S"), "hidden content must not paint");
}

#[test]
fn test_open_details_renders_all_children() {
    init();
    let (boxes, arena) =
        layout_boxes("<details open><summary>More</summary><p>Shown</p></details>");
    assert!(find_box_by_type(&boxes, &NodeType::Summary, &arena).is_some());
    let p = find_box_by_type(&boxes, &NodeType::P, &arena).expect("open details shows content");
    assert_eq!(
        p.rect.y, 2,
        "content starts below the summary row + its 1-row UA margin-top"
    );
    let (_, tl) = draw("<details open><summary>More</summary><p>Shown</p></details>");
    assert!(
        canvas_has_word(&tl, "▾"),
        "open details shows the open marker"
    );
    assert!(canvas_has_word(&tl, "S"), "content paints when open");
}

#[test]
fn test_closed_details_without_summary_renders_nothing() {
    init();
    // No `<summary>` child at all: a closed details hides everything, and an
    // open one shows it all.
    let (boxes, arena) = layout_boxes("<details><p>Hidden</p></details>");
    assert!(
        find_box_by_type(&boxes, &NodeType::P, &arena).is_none(),
        "closed details without a summary renders nothing"
    );
    let (boxes, arena) = layout_boxes("<details open><p>Shown</p></details>");
    assert!(
        find_box_by_type(&boxes, &NodeType::P, &arena).is_some(),
        "open details without a summary still renders content"
    );
}

#[test]
fn test_closed_details_keeps_only_the_first_summary() {
    init();
    // Content before the summary is hidden too; a second `<summary>` is
    // hidden exactly like any other non-first child (browsers only special
    // case the first summary).
    let html = "<details><p>before</p><summary>Toggle</summary><p>between</p><summary>again</summary></details>";
    let (boxes, arena) = layout_boxes(html);
    let summary =
        find_box_by_type(&boxes, &NodeType::Summary, &arena).expect("first summary renders");
    let mut summary_count = 0;
    collect_box_count(&boxes, &NodeType::Summary, &arena, &mut summary_count);
    assert_eq!(
        summary_count, 1,
        "only the first summary renders when closed"
    );
    assert!(summary.rect.height == 1);
    assert!(
        find_box_by_type(&boxes, &NodeType::P, &arena).is_none(),
        "non-summary content stays hidden"
    );
    let (_, tl) = draw(html);
    assert!(canvas_has_word(&tl, "T"));
    assert!(
        !canvas_has_word(&tl, "f"),
        "content before the summary stays hidden"
    );
    assert!(
        !canvas_has_word(&tl, "w"),
        "content after the summary stays hidden"
    );
    assert!(
        !canvas_has_word(&tl, "i"),
        "the second summary stays hidden"
    );
}

fn collect_box_count(
    boxes: &[htui::layout::LayoutBox],
    target: &NodeType,
    arena: &NodeArena,
    out: &mut usize,
) {
    for b in boxes {
        if &arena[b.node_id].node_type == target {
            *out += 1;
        }
        collect_box_count(&b.children, target, arena, out);
    }
}

#[test]
fn test_summary_outside_details_has_no_marker() {
    init();
    // A standalone `<summary>` is a plain block — the disclosure marker only
    // exists inside a `<details>` (browser UA `summary::marker`).
    let (_, tl) = draw("<summary>plain</summary>");
    assert!(canvas_has_word(&tl, "p"));
    assert!(
        !canvas_has_word(&tl, "▸"),
        "no disclosure marker outside details"
    );
    assert!(!canvas_has_word(&tl, "▾"));
}

#[test]
fn test_summary_marker_styles_with_css() {
    init();
    // The marker run carries the summary's own fg / bg / font style.
    let (_, tl) = draw("<details><summary>More</summary></details>");
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "▸")
        .expect("marker cell");
    assert_eq!(
        tl.layout[idx].fcolor, None,
        "default fg is the terminal's own"
    );
    let (_, tl) =
        draw("<style>summary { color: blue; }</style><details><summary>More</summary></details>");
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "▸")
        .expect("marker cell");
    let blue = Color::Rgb { r: 0, g: 0, b: 255 };
    assert_eq!(
        tl.layout[idx].fcolor,
        Some(blue),
        "marker inherits summary color"
    );
}

#[test]
fn test_clicking_summary_toggles_open_and_fires_toggle() {
    init();
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var toggles = [];
          var d = document.getElementById('d');
          d.addEventListener('toggle', function () {
            toggles.push(d.getAttribute('open') === null ? 'closed' : 'open');
          });
          globalThis.__toggles = toggles;
        </script>
        <details id="d"><summary>More</summary><p>Secret</p></details>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("d").expect("details region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let ev = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    assert!(
        engine.arena()[region.node_id]
            .attributes
            .contains_key("open"),
        "clicking the summary must set the open attribute"
    );
    assert!(ev.dirty.is_some(), "the toggle marks the details dirty");
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__toggles.join(','))"),
        "open",
        "toggle event fires after the state flip with the new state"
    );

    // Second click closes it again.
    let _ev = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    assert!(
        !engine.arena()[region.node_id]
            .attributes
            .contains_key("open"),
        "second click removes the open attribute"
    );
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__toggles.join(','))"),
        "open,closed",
        "every state flip fires exactly one toggle event"
    );
}

#[test]
fn test_clicking_summary_content_toggles_the_details() {
    init();
    // The activation target is a box-producing descendant of the summary (a
    // block-level `<div>`): the default action still resolves to the owning
    // details by walking up the DOM.
    let (mut engine, geo) = build_geometry(
        r#"<details id="d"><summary>More<div id="s">Block</div></summary><p>Secret</p></details>"#,
    );
    let map = HitMap::build(&geo);
    let div = map.resolve_by_id("s").expect("div region");
    let (x, y) = center(div.rect);

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    let details_id = find_node_by_id(&engine.arena(), "d");
    assert!(
        engine.arena()[details_id].attributes.contains_key("open"),
        "clicking content inside the summary toggles the details"
    );
}

#[test]
fn test_clicking_details_content_does_not_toggle() {
    init();
    // Clicking the expanded content itself (not the summary) must not flip
    // the disclosure state — only the summary has the activation behavior.
    let (mut engine, geo) = build_geometry(
        r#"<details id="d" open><summary>More</summary><p id="c">Secret</p></details>"#,
    );
    let map = HitMap::build(&geo);
    let content = map.resolve_by_id("c").expect("content region");
    let (x, y) = center(content.rect);

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    let details_id = find_node_by_id(&engine.arena(), "d");
    assert!(
        engine.arena()[details_id].attributes.contains_key("open"),
        "clicking expanded content keeps the details open"
    );
}

#[test]
fn test_toggle_mutation_reflows_to_reveal_content() {
    init();
    // After the toggle marks the details dirty, a re-layout from the engine's
    // live arena must show the newly revealed content (and the open marker).
    let (mut engine, geo) =
        build_geometry(r#"<details id="d"><summary>More</summary><p>Secret</p></details>"#);
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("d").expect("details region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });

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
    assert!(
        find_box_by_type(&boxes, &NodeType::P, &arena).is_some(),
        "content appears after opening"
    );
    assert!(
        find_box_by_type(&boxes, &NodeType::Summary, &arena).is_some(),
        "summary stays visible"
    );
}

#[test]
fn test_ontoggle_attribute_runs_on_click() {
    init();
    // `ontoggle` compiles to a listener exactly like the other on* events.
    let (mut engine, geo) = build_geometry(
        r#"<script>
          var seen = [];
          globalThis.__seen = seen;
        </script>
        <details id="d" ontoggle="seen.push(event.type)"><summary>More</summary><p>x</p></details>"#,
    );
    let map = HitMap::build(&geo);
    let region = map.resolve_by_id("d").expect("details region");
    let (x, y) = center(region.rect);

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    assert_eq!(
        eval_str(&mut engine, "String(globalThis.__seen.join(','))"),
        "toggle",
        "ontoggle attribute fires on the details"
    );
}

#[test]
fn test_closed_details_hides_own_inline_text() {
    init();
    // Text directly inside a closed details (outside any summary) belongs to
    // the hidden children — the element's own inline runs must be filtered
    // just like its block children.
    let (_, tl) = draw("<details>secret text</details>");
    let painted: Vec<&str> = tl
        .layout
        .iter()
        .filter(|b| !b.word.trim().is_empty())
        .map(|b| b.word.as_str())
        .collect();
    assert!(
        painted.is_empty(),
        "a closed details with no summary paints no text: {painted:?}"
    );
    let (boxes, arena) = layout_boxes("<details>lead<summary>s</summary>tail</details>");
    assert!(
        find_box_by_type(&boxes, &NodeType::Summary, &arena).is_some(),
        "the summary still renders"
    );
    let (_, tl) = draw("<details>lead<summary>s</summary>tail</details>");
    assert!(canvas_has_word(&tl, "s"), "summary text paints");
    assert!(
        !canvas_has_word(&tl, "l"),
        "text before the summary stays hidden"
    );
    assert!(
        !canvas_has_word(&tl, "t"),
        "text after the summary stays hidden"
    );
}

#[test]
fn test_open_details_shows_own_inline_text() {
    init();
    let (_, tl) = draw("<details open>visible text</details>");
    assert!(canvas_has_word(&tl, "v"), "open details paints its text");
}

#[test]
fn visual_test_details_document_initializes() {
    init();
    // Smoke test mirroring `visual_test_events_document_initializes`: the
    // demo page's `<script>` block must initialize in the engine (toggle
    // listeners registered) without error, and clicking the summary must
    // drive the native disclosure-widget end-to-end — `open` flips, the
    // `toggle` event fires, and the page's own listener updates the status
    // line.
    let html = include_str!("../../../visual_test/src/details/index.html");
    let (mut engine, geo) = build_geometry(html);
    let map = HitMap::build(&geo);
    let d1 = find_node_by_id(&engine.arena(), "d1");
    let region = map.resolve_by_id("d1").expect("d1 hit region");
    let (x, y) = center(region.rect);

    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "— click a summary —",
        "the demo boots with its initial status line"
    );

    let mut hover = HoverState::new();
    let _ = apply_message(&mut engine, &mut hover, &map, ScriptMsg::Click { x, y });
    assert!(
        engine.arena()[d1].attributes.contains_key("open"),
        "clicking the demo summary must open the details"
    );
    assert_eq!(
        eval_str(&mut engine, "document.getElementById('status').textContent"),
        "d1 opened",
        "the demo's toggle listener reports the state flip"
    );
}

// ============================================================
// Helpers
// ============================================================

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

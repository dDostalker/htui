//! Tests for `<em>` (emphasis) inline rendering.
//!
//! `<em>` is recognised by the HTML parser and the selector parser, but
//! previously had no handler in the layout pass — its text content was
//! silently dropped because `walk_runs` only collected `Text`, `<span>` and
//! `<a>` children. These tests pin down the inline rendering: em text
//! reaches the canvas, the user-agent default (`font-style: italic`) applies
//! when no CSS overrides it, and author CSS overrides take precedence over
//! the default (e.g. `em { font-style: normal }` or `em { color: red }`).
//!
//! The parser-level mapping (`<em>` → `NodeType::Em`) and CSS selector
//! matching (`em { … }`) are covered by the generic tag-selector tests in
//! `html_parse_tests` / `css_parse_tests`; here we focus on the layout-time
//! behaviour so regressions in `walk_runs` are caught.

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tui::graph_api::{FontStyle, TermLayout};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn setup(html: &str, css: &str) -> (htui::tdom::node::NodeId, htui::tdom::node::NodeArena) {
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
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

fn canvas_string(tl: &TermLayout) -> String {
    tl.layout.iter().map(|b| b.word.as_str()).collect()
}

fn first_cell_with(tl: &TermLayout, word: &str) -> Option<usize> {
    tl.layout.iter().position(|b| b.word == word)
}

#[test]
fn test_em_renders_text_content() {
    // The emphasised text must reach the canvas. Before this was implemented
    // the text inside `<em>` was silently dropped by `walk_runs` because the
    // parser flattened unknown inline tags.
    init();
    let (root_id, arena) = setup("<p>plain <em>italic</em> tail</p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas = canvas_string(&tl);
    assert!(canvas.contains("plain"), "pre-em text must render");
    assert!(canvas.contains("italic"), "em text must render");
    assert!(canvas.contains("tail"), "post-em text must render");
}

#[test]
fn test_em_default_italic_applies() {
    // UA default: an unstyled `<em>` is italic.
    init();
    let (root_id, arena) = setup("<p><em>word</em></p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "w").expect("em text must render");
    let cell = &tl.layout[idx];
    assert!(
        cell.style.contains(FontStyle::ITALIC),
        "default <em> rendering must be italic (got style {:?})",
        cell.style
    );
}

#[test]
fn test_em_default_italic_does_not_leak_outside_em() {
    // The italic flag must stay scoped to the em's text — surrounding
    // siblings keep the inherited (non-italic) style. Use `f` (only inside
    // em) and `x` (only outside) so the cell lookup is unambiguous.
    init();
    let (root_id, arena) = setup("<p>xxx <em>foo</em> yyy</p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let em_idx = first_cell_with(&tl, "f").expect("em text must render");
    let plain_idx = first_cell_with(&tl, "x").expect("pre-em text must render");
    assert!(tl.layout[em_idx].style.contains(FontStyle::ITALIC));
    assert!(
        !tl.layout[plain_idx].style.contains(FontStyle::ITALIC),
        "text outside <em> must not inherit the UA italic flag (got {:?})",
        tl.layout[plain_idx].style
    );
}

#[test]
fn test_em_font_style_normal_disables_italic() {
    // Author CSS resetting the UA default must take effect.
    init();
    let (root_id, arena) = setup("<p><em>word</em></p>", "em { font-style: normal; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "w").expect("em text must render");
    let cell = &tl.layout[idx];
    assert!(
        !cell.style.contains(FontStyle::ITALIC),
        "`em {{ font-style: normal }}` must suppress the UA italic default (got {:?})",
        cell.style
    );
}

#[test]
fn test_em_color_override_applies() {
    // A `color` rule on `<em>` must reach the canvas cell while the UA italic
    // default still applies (no `font-style` declared to suppress it).
    init();
    let (root_id, arena) = setup("<p><em>word</em></p>", "em { color: magenta; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "w").expect("em text must render");
    let cell = &tl.layout[idx];
    assert_eq!(
        cell.fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 255
        }),
        "`em {{ color: magenta }}` must paint the cell magenta (got {:?})",
        cell.fcolor
    );
    assert!(
        cell.style.contains(FontStyle::ITALIC),
        "color override must not affect the italic UA default"
    );
}

#[test]
fn test_em_nested_in_p_renders_correctly() {
    // Exercise `<em>` inline inside a paragraph among other text. Confirm
    // the em tag survives and its italic flag is applied while the
    // surrounding text is not italic.
    // Use `l` (only in em "inline" and after) and `A` (only in pre-em
    // "Another") to disambiguate which cell we're inspecting.
    init();
    let (root_id, arena) = setup(
        "<p>Another paragraph with <em>inline</em> elements (em tags are flattened).</p>",
        "",
    );
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas = canvas_string(&tl);
    assert!(
        canvas.contains("inline"),
        "em text must survive in the rendered canvas"
    );

    // The first 'l' in the canvas is inside em ("inline").
    let em_idx = first_cell_with(&tl, "l").expect("em text must render");
    assert!(tl.layout[em_idx].style.contains(FontStyle::ITALIC));
    let another_idx = first_cell_with(&tl, "A").expect("pre-em text must render");
    assert!(!tl.layout[another_idx].style.contains(FontStyle::ITALIC));
}

#[test]
fn test_em_inside_div_paint_layer() {
    // `<em>` is also reachable through the div layout path (different from
    // `<p>`). Confirm the same UA italic default applies.
    init();
    let (root_id, arena) = setup("<div>before <em>italic-here</em> after</div>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "i").expect("em text must render");
    assert!(
        tl.layout[idx].style.contains(FontStyle::ITALIC),
        "<em> inside <div> must receive the UA italic default (got {:?})",
        tl.layout[idx].style
    );
}

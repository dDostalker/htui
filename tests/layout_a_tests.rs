//! Tests for `<a>` (anchor / hyperlink) inline rendering.
//!
//! `<a>` is recognised by the parser but previously had no handler in the
//! layout pass — its text content was silently dropped because `walk_runs`
//! only collected `Text` and `<span>` children. These tests pin down the
//! inline rendering: link text reaches the canvas, the user-agent defaults
//! (blue foreground, underline) apply when no CSS overrides them, and
//! author CSS overrides take precedence over the defaults.

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
fn test_a_renders_text_content() {
    // The link text must reach the canvas. Before this was implemented the
    // text inside `<a>` was silently dropped by `walk_runs`.
    init();
    let (root_id, arena) = setup("<p>click <a href=\"https://example.com\">here</a> now</p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas = canvas_string(&tl);
    assert!(canvas.contains("click"), "pre-link text must render");
    assert!(canvas.contains("here"), "link text must render");
    assert!(canvas.contains("now"), "post-link text must render");
}

#[test]
fn test_a_default_underline_applies() {
    // UA default: an unstyled link is underlined.
    init();
    let (root_id, arena) = setup("<p><a href=\"x\">link</a></p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert!(
        cell.style.contains(FontStyle::UNDERLINED),
        "default <a> rendering must be underlined (got style {:?})",
        cell.style
    );
}

#[test]
fn test_a_default_blue_color_applies() {
    // UA default: an unstyled link is blue.
    init();
    let (root_id, arena) = setup("<p><a href=\"x\">link</a></p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert_eq!(
        cell.fcolor,
        Some(Color::Blue),
        "default <a> rendering must use blue foreground (got {:?})",
        cell.fcolor
    );
}

#[test]
fn test_a_text_decoration_none_disables_underline() {
    // Author CSS overriding the UA default must take effect.
    init();
    let (root_id, arena) = setup(
        "<p><a href=\"x\">link</a></p>",
        "a { text-decoration: none; }",
    );
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert!(
        !cell.style.contains(FontStyle::UNDERLINED),
        "`a {{ text-decoration: none }}` must suppress the UA underline (got {:?})",
        cell.style
    );
}

#[test]
fn test_a_color_override_disables_default_blue() {
    init();
    let (root_id, arena) = setup(
        "<p><a href=\"x\">link</a></p>",
        "a { color: magenta; }",
    );
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert_eq!(
        cell.fcolor,
        Some(Color::Magenta),
        "`a {{ color: magenta }}` must override the default blue"
    );
    // Underline default should still apply because we didn't override it.
    assert!(
        cell.style.contains(FontStyle::UNDERLINED),
        "color override must not affect the underline default"
    );
}

#[test]
fn test_a_inherits_parent_color_when_no_override() {
    // When no `color` is declared on the link or any ancestor that set
    // `color`, the UA blue default applies. The parent block's `color: red`
    // does propagate through `walk_runs`' `fg` parameter, so the link must
    // render red (inherited) rather than blue (UA default).
    init();
    let (root_id, arena) = setup(
        "<p><a href=\"x\">link</a></p>",
        "p { color: red; }",
    );
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert_eq!(
        cell.fcolor,
        Some(Color::Red),
        "link must inherit the parent block's color (got {:?})",
        cell.fcolor
    );
}

#[test]
fn test_a_inside_div_with_padding_and_bg() {
    // Regression guard: `<a>` rendered inside a styled `<div>` still hits
    // the canvas and keeps its UA underline. The bg of the div should
    // bleed through to the link cells.
    init();
    let html = "<div><a href=\"x\">link</a></div>";
    let css = "div { background-color: yellow; padding: 1px; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas = canvas_string(&tl);
    assert!(canvas.contains("link"), "<a> inside <div> must render");
    let idx = first_cell_with(&tl, "l").expect("link cell exists");
    assert!(
        tl.layout[idx].style.contains(FontStyle::UNDERLINED),
        "link should keep its UA underline inside the div"
    );
}

#[test]
fn test_a_nested_in_span_keeps_underline_default() {
    // The link's UA underline default applies whether or not the link sits
    // inside another inline wrapper — the surrounding `<span>` does not
    // suppress the default.
    init();
    let (root_id, arena) = setup(
        "<p><span>prefix <a href=\"x\">link</a> suffix</span></p>",
        "",
    );
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert!(
        cell.style.contains(FontStyle::UNDERLINED),
        "<a> nested inside <span> must still receive the UA underline"
    );
}

#[test]
fn test_a_empty_link_does_not_paint() {
    // An empty `<a>` (no text children) should not produce any run. This
    // is the negative case to confirm we don't accidentally emit empty
    // glyphs for the link tag itself.
    init();
    let (root_id, arena) = setup("<p>before<a href=\"x\"></a>after</p>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let canvas = canvas_string(&tl);
    assert!(canvas.contains("before"));
    assert!(canvas.contains("after"));
    // No cell should have an underline since the `<a>` contributed nothing.
    assert!(
        tl.layout.iter().all(|b| !b.style.contains(FontStyle::UNDERLINED)),
        "empty <a> must not add any underlined cells"
    );
}

#[test]
fn test_a_preserves_text_decoration_line_through_instead_of_underline() {
    // When the author sets `line-through`, `block_style` returns CROSSED_OUT
    // and the UA-default-underline branch is skipped (is_set is true). The
    // link should render with line-through, not both decorations.
    init();
    let (root_id, arena) = setup(
        "<p><a href=\"x\">link</a></p>",
        "a { text-decoration: line-through; }",
    );
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert!(
        cell.style.contains(FontStyle::CROSSED_OUT),
        "link must pick up the declared line-through"
    );
    assert!(
        !cell.style.contains(FontStyle::UNDERLINED),
        "UA underline must yield to the declared line-through"
    );
}
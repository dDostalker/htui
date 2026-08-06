//! Tests for `<a>` (anchor / hyperlink) rendering.
//!
//! `<a>` participates in two layout paths:
//!
//! - **Inline** when it sits inside another block's inline flow
//!   (e.g. `<p>click <a>here</a> now</p>`). Handled by `walk_runs`; the
//!   user-agent defaults (blue foreground, underline) and author-CSS
//!   overrides are tested below in the *inline* section.
//! - **Block** when it sits as a direct block-level child
//!   (e.g. `<aside><a>Dashboard</a></aside>`). Handled by
//!   [`crate::layout::layoutbox::inner_formatting_context::flow::layout_a`].
//!   The user-agent defaults still apply; the *block* section below pins
//!   down the new behaviour and guards against the silent regression that
//!   left `<a>` unrendered before `layout_a` was registered.

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
    tl.draw(&arena, root_id, &term_map);
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
    tl.draw(&arena, root_id, &term_map);
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
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert_eq!(
        cell.fcolor,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 255
        }),
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
    tl.draw(&arena, root_id, &term_map);
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
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert_eq!(
        cell.fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 255
        }),
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
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    let cell = &tl.layout[idx];
    assert_eq!(
        cell.fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 0
        }),
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
    tl.draw(&arena, root_id, &term_map);
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
    tl.draw(&arena, root_id, &term_map);
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
    tl.draw(&arena, root_id, &term_map);
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
    tl.draw(&arena, root_id, &term_map);
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

// =============================================================================
// Block-level rendering
// =============================================================================

/// Build the canvas and return the per-row text string for inspection. Mirrors
/// the helper used by `layout_flex_tests` so block-position assertions have the
/// same shape as the existing flex coverage.
fn render_rows(html: &str, css: &str) -> (Vec<String>, TermLayout) {
    let (root_id, arena) = setup(html, css);
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

#[test]
fn test_block_a_renders_text_as_its_own_row() {
    // Before `layout_a` was registered, an `<a>` sitting as a direct block-level
    // child was silently dropped (the registry iterated past it and returned
    // `None`). This is the regression guard: the four `<a>` items in a
    // navigation-style sidebar must each occupy their own row, in document
    // order, with no text loss.
    init();
    let html = r##"
        <aside>
            <h3>Menu</h3>
            <a href="#">Dashboard</a>
            <a href="#">Projects</a>
            <a href="#">Tasks</a>
            <a href="#">Settings</a>
        </aside>
    "##;
    let (rows, _) = render_rows(html, "a { display: block; }");
    assert!(rows.iter().any(|r| r.contains("Dashboard")), "Dashboard must render");
    assert!(rows.iter().any(|r| r.contains("Projects")), "Projects must render");
    assert!(rows.iter().any(|r| r.contains("Tasks")), "Tasks must render");
    assert!(rows.iter().any(|r| r.contains("Settings")), "Settings must render");
}

#[test]
fn test_block_a_default_underline_applies() {
    // UA default for the block path: an unstyled block `<a>` is underlined.
    init();
    let html = r#"<aside><a href="x">link</a></aside>"#;
    let (root_id, arena) = setup(html, "a { display: block; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    assert!(
        tl.layout[idx].style.contains(FontStyle::UNDERLINED),
        "block <a> UA default must be underlined (got {:?})",
        tl.layout[idx].style
    );
}

#[test]
fn test_block_a_default_blue_color_applies() {
    // UA default for the block path: an unstyled block `<a>` is blue.
    init();
    let html = r#"<aside><a href="x">link</a></aside>"#;
    let (root_id, arena) = setup(html, "a { display: block; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    assert_eq!(
        tl.layout[idx].fcolor,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 255
        }),
        "block <a> UA default must use blue foreground (got {:?})",
        tl.layout[idx].fcolor
    );
}

#[test]
fn test_block_a_text_decoration_none_disables_underline() {
    // Author CSS overriding the UA default must take effect on the block path.
    init();
    let html = r#"<aside><a href="x">link</a></aside>"#;
    let (root_id, arena) = setup(
        html,
        "a { display: block; text-decoration: none; }",
    );
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    assert!(
        !tl.layout[idx].style.contains(FontStyle::UNDERLINED),
        "`a {{ text-decoration: none }}` must suppress the UA underline on the block path"
    );
}

#[test]
fn test_block_a_color_override_disables_default_blue() {
    init();
    let html = r#"<aside><a href="x">link</a></aside>"#;
    let (root_id, arena) = setup(html, "a { display: block; color: magenta; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    assert_eq!(
        tl.layout[idx].fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 255
        }),
        "`a {{ color: magenta }}` must override the default blue on the block path"
    );
    // Underline default should still apply because we didn't override it.
    assert!(
        tl.layout[idx].style.contains(FontStyle::UNDERLINED),
        "color override must not affect the underline default on the block path"
    );
}

#[test]
fn test_block_a_inherits_parent_color_when_no_override() {
    // The cascade populates `color` for `<a>` from its parent block; the
    // cloned-Node approach in `layout_a` must not lose that inherited value.
    init();
    let html = r#"<aside><a href="x">link</a></aside>"#;
    let (root_id, arena) = setup(html, "aside { color: red; } a { display: block; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = first_cell_with(&tl, "l").expect("link text must render");
    assert_eq!(
        tl.layout[idx].fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 0
        }),
        "block <a> must inherit the parent block's color (got {:?})",
        tl.layout[idx].fcolor
    );
}

#[test]
fn test_block_a_visual_test_sidebar_renders_all_menu_items() {
    // End-to-end regression for the `css` visual test: the sidebar's `<a>`
    // menu items (Dashboard / Projects / Tasks / Settings) must each appear
    // in the canvas. Before `layout_a` was registered, only the `<h3>Menu`
    // heading rendered — the four `<a>` links were silently dropped.
    init();
    let html = r##"
        <aside class="sidebar">
            <h3>Menu</h3>
            <a href="#">Dashboard</a>
            <a href="#">Projects</a>
            <a href="#">Tasks</a>
            <a href="#">Settings</a>
        </aside>
    "##;
    let css = r#"
        .sidebar a {
            display: block;
            color: #cbd5e1;
            text-decoration: none;
            padding: 8px 0;
        }
    "#;
    let (rows, tl) = render_rows(html, css);
    for menu_item in ["Dashboard", "Projects", "Tasks", "Settings"] {
        assert!(
            rows.iter().any(|r| r.contains(menu_item)),
            "sidebar <a> menu item {menu_item:?} must render"
        );
    }
    // Author `text-decoration: none` overrides the UA underline default:
    // no link cell should carry UNDERLINED.
    let any_underlined = tl.layout.iter().any(|b| b.style.contains(FontStyle::UNDERLINED));
    assert!(
        !any_underlined,
        "author `text-decoration: none` must suppress the UA underline default on every sidebar link"
    );
}
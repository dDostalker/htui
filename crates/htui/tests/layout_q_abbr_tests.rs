//! `<q>` / `<abbr>` tests — inline text-level elements.
//!
//! - `<q>` wraps its content in generated quote marks (UA `::before` /
//!   `::after`); the pair alternates with nesting depth like the CSS
//!   `quotes` property (double → single).
//! - `<abbr>` is plain inline text; when it carries a `title` attribute the
//!   browser UA dotted underline is approximated with a solid underline,
//!   and an explicit `text-decoration` wins.

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{FontStyle, TermLayout};

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

/// Render the document onto the terminal canvas.
fn draw(html: &str, css: &str) -> (TermMap, TermLayout) {
    let term_map = mock_term_map();
    let (root_id, arena) = setup(html, css);
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    (term_map, tl)
}

/// Index of the first canvas cell carrying `word`.
fn first_cell_with(tl: &TermLayout, word: &str) -> Option<usize> {
    tl.layout.iter().position(|b| b.word == word)
}

#[test]
fn test_parse_tag_map_q_abbr() {
    init();
    assert_eq!(html_parse::tag_name_to_node_type("q"), Some(NodeType::Q));
    assert_eq!(
        html_parse::tag_name_to_node_type("abbr"),
        Some(NodeType::Abbr)
    );
}

#[test]
fn test_q_wraps_content_in_quotes() {
    init();
    // `<q>` synthesises the UA `::before`/`::after` quote marks: the content
    // sits between `“` and `”` on the same line.
    let (term_map, tl) = draw("<p>He said <q>hi</q>.</p>", "");
    let cols = term_map.columns as usize;
    let open = first_cell_with(&tl, "\u{201C}").expect("open quote should render");
    let close = first_cell_with(&tl, "\u{201D}").expect("close quote should render");
    let hi = first_cell_with(&tl, "h").expect("q content should render");
    assert_eq!(
        open / cols,
        1,
        "quote sits on the paragraph's text row (UA margin-top=1)"
    );
    assert_eq!(open % cols, 8, "quote opens after `He said `");
    assert_eq!(hi, open + 1, "content follows the open quote");
    assert_eq!(close, hi + 2, "close quote follows the content");
    assert_eq!(close % cols, 11);
    assert_eq!(
        first_cell_with(&tl, ".").expect("period should render"),
        close + 1,
        "sentence continues after the close quote"
    );
}

#[test]
fn test_q_nested_alternates_quote_pairs() {
    init();
    // Nesting depth switches the pair to single quotes (CSS `quotes`
    // alternation): `“outer ‘inner’”`.
    let (_, tl) = draw("<p><q>outer <q>inner</q></q></p>", "");
    let double_open = first_cell_with(&tl, "\u{201C}").expect("outer open quote");
    let double_close = first_cell_with(&tl, "\u{201D}").expect("outer close quote");
    let single_open = first_cell_with(&tl, "\u{2018}").expect("inner open quote");
    let single_close = first_cell_with(&tl, "\u{2019}").expect("inner close quote");
    assert!(
        double_open < single_open && single_open < single_close && single_close < double_close,
        "outer double quotes must wrap the inner single-quoted span \
         (got {double_open} < {single_open} < {single_close} < {double_close})"
    );
}

#[test]
fn test_q_author_color_applies_to_quote_marks() {
    init();
    // The generated quote runs carry the `<q>` element's own style, so the
    // author colour reaches the quote marks too.
    let (_, tl) = draw("<p><q>red</q></p>", "q { color: red; }");
    let open = first_cell_with(&tl, "\u{201C}").expect("open quote should render");
    assert_eq!(
        tl.layout[open].fcolor,
        Some(Color::Rgb { r: 255, g: 0, b: 0 }),
        "quote marks must inherit the q element's colour"
    );
}

#[test]
fn test_q_stays_inline_flow() {
    init();
    // `<q>` is inline: the whole sentence (quotes included) stays on the
    // paragraph's single line rather than starting its own block.
    let (term_map, tl) = draw("<p>Before <q>mid</q> after</p>", "");
    let cols = term_map.columns as usize;
    let before = first_cell_with(&tl, "B").expect("pre-q text should render");
    let after = first_cell_with(&tl, "a").expect("post-q text should render");
    assert_eq!(before / cols, after / cols, "all text shares one row");
}

#[test]
fn test_abbr_plain_without_title_not_underlined() {
    init();
    let (_, tl) = draw("<p><abbr>HTML</abbr></p>", "");
    let idx = first_cell_with(&tl, "H").expect("abbr text should render");
    assert!(
        !tl.layout[idx].style.contains(FontStyle::UNDERLINED),
        "plain <abbr> (no title) must not be underlined"
    );
}

#[test]
fn test_abbr_with_title_underlined() {
    init();
    // Browser UA `abbr[title] { text-decoration: underline dotted }` — the
    // terminal approximation is a solid underline.
    let (_, tl) = draw(
        "<p><abbr title=\"HyperText Markup Language\">HTML</abbr></p>",
        "",
    );
    let idx = first_cell_with(&tl, "H").expect("abbr text should render");
    assert!(
        tl.layout[idx].style.contains(FontStyle::UNDERLINED),
        "abbr[title] must render underlined (got {:?})",
        tl.layout[idx].style
    );
}

#[test]
fn test_abbr_title_underline_yields_to_author_decoration() {
    init();
    let (_, tl) = draw(
        "<p><abbr title=\"HyperText Markup Language\">HTML</abbr></p>",
        "abbr { text-decoration: none; }",
    );
    let idx = first_cell_with(&tl, "H").expect("abbr text should render");
    assert!(
        !tl.layout[idx].style.contains(FontStyle::UNDERLINED),
        "author text-decoration must suppress the UA underline"
    );
}

#[test]
fn test_abbr_type_selector_color_with_underline() {
    init();
    let (_, tl) = draw(
        "<p><abbr title=\"HyperText Markup Language\">HTML</abbr></p>",
        "abbr { color: blue; }",
    );
    let idx = first_cell_with(&tl, "H").expect("abbr text should render");
    let cell = &tl.layout[idx];
    assert_eq!(cell.fcolor, Some(Color::Rgb { r: 0, g: 0, b: 255 }));
    assert!(
        cell.style.contains(FontStyle::UNDERLINED),
        "colour override must not affect the title underline default"
    );
}

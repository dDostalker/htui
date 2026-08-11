use htui::html::html_parse::Parser;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId};
use htui::tui::graph_api::{FontStyle, TermLayout};
use crossterm::style::Color;

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

#[test]
fn test_layout_p() {
    let mut p = Parser::new("<p>Hello</p><p>World</p>");
    let (root, arena, _) = p.parse();
    let term_map = TermMap {
        columns: 80,
        rows: 24,
    };
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Hello"));
    assert!(canvas.contains("World"));
}

// ============================================================
// text-align — exercised through `<p>` and `<h1>`-`<h5>` so the
// inheritance chain `div { text-align: ... } → <p>` is covered.
// ============================================================

#[test]
fn test_layout_p_text_align_center() {
    init();
    let html = "<p>Centered</p>";
    let css = "p { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let row = text_idx / cols;
    let rstart = row * cols;
    let leading_spaces = tl.layout[rstart..rstart + cols]
        .iter()
        .take_while(|b| b.word == " ")
        .count();
    let text_str_width = "Centered".len();
    let expected_padding = (cols - text_str_width) / 2;
    assert_eq!(
        leading_spaces, expected_padding,
        "<p> text should be centered within row"
    );
}

#[test]
fn test_layout_p_text_align_right() {
    init();
    let html = "<p>Right</p>";
    let css = "p { text-align: right; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let row = text_idx / cols;
    let rstart = row * cols;
    let leading_spaces = tl.layout[rstart..rstart + cols]
        .iter()
        .take_while(|b| b.word == " ")
        .count();
    let text_str_width = "Right".len();
    let expected_padding = cols - text_str_width;
    assert_eq!(
        leading_spaces, expected_padding,
        "<p> text should be right-aligned within row"
    );
}

#[test]
fn test_layout_p_text_align_inherited_from_div() {
    init();
    let html = "<div><p>centered</p></div>";
    let css = "div { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    // "centered" must not start at column 0 — it should be indented by the
    // center-padding because <p> inherited text-align from <div>.
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "c")
        .expect("should find 'centered'");
    let row = text_idx / cols;
    let col_in_row = text_idx % cols;
    let text_str_width = "centered".len();
    let expected_padding = (cols - text_str_width) / 2;
    assert_eq!(
        col_in_row, expected_padding,
        "<p> should inherit text-align from <div>, row={row}"
    );
}

#[test]
fn test_layout_h1_text_align_center() {
    init();
    let html = "<h1>Title</h1>";
    let css = "h1 { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    // Rendered text is "# Title" (prefix included in alignment width).
    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("should find heading '#' prefix");
    let col_in_row = text_idx % cols;
    let rendered_width = "# Title".chars().count() * 2;
    let expected_padding = (cols - rendered_width) / 2;
    assert_eq!(
        col_in_row, expected_padding,
        "<h1> should be centered including '# ' prefix"
    );
}

#[test]
fn test_layout_h2_text_align_right() {
    init();
    let html = "<h2>Sub</h2>";
    let css = "h2 { text-align: right; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("should find heading '#' prefix");
    let col_in_row = text_idx % cols;
    let rendered_width = "## Sub".chars().count() * 2;
    let expected_padding = cols - rendered_width;
    assert_eq!(
        col_in_row, expected_padding,
        "<h2> should be right-aligned including '## ' prefix"
    );
}

#[test]
fn test_layout_heading_text_align_inherited_from_div() {
    init();
    let html = "<div><h1>Heading</h1></div>";
    let css = "div { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("should find heading prefix");
    let col_in_row = text_idx % cols;
    let rendered_width = "# Heading".chars().count() * 2;
    let expected_padding = (cols - rendered_width) / 2;
    assert_eq!(
        col_in_row, expected_padding,
        "<h1> should inherit text-align from <div>"
    );
}

#[test]
fn test_layout_p_text_align_default_left() {
    init();
    let html = "<p>Left</p>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let text_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " ")
        .expect("should find text");
    let row = text_idx / cols;
    let col_in_row = text_idx % cols;
    assert_eq!(
        col_in_row, 0,
        "default text-align should leave text at column 0, row={row}"
    );
}

// ============================================================
// Terminal-size sanity bounds
// ============================================================

#[test]
fn term_layout_caps_a_pathological_viewport_size() {
    // A transiently-inverted Windows `SMALL_RECT` during a resize can make
    // crossterm report ~65535 columns / rows (i16 → u16 wrap). Taking that
    // literally would allocate `65535 × 65535` cells (~hundreds of GB) and
    // freeze the render thread — `TermLayout::new` must cap the canvas.
    let term_map = TermMap {
        columns: u16::MAX,
        rows: u16::MAX,
    };
    let tl = TermLayout::new(&term_map, 1.0);
    let max_cells = htui::layout::layout::MAX_TERM_COLS as usize
        * htui::layout::layout::MAX_TERM_ROWS as usize;
    assert!(
        tl.layout.len() <= max_cells,
        "canvas must be bounded by the size clamps, got {}",
        tl.layout.len()
    );
}

#[test]
fn term_layout_caps_an_extreme_but_plausible_viewport() {
    // 3000 × 1000 would still be a ~96 MB allocation with 64-byte cells; the
    // clamp must keep even an out-of-spec size within the sane bound.
    let term_map = TermMap {
        columns: 3000,
        rows: 1000,
    };
    let tl = TermLayout::new(&term_map, 1.0);
    assert!(tl.layout.len() <= 1000 * 500);
}

// ============================================================
// <h6> / <hgroup> / <address>
// ============================================================

#[test]
fn test_layout_h6_prefix_renders() {
    init();
    let html = "<h6>Small</h6>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    // h6's default `0.67em` rounds to 1 cell per char (like h4/h5), so the
    // `###### ` prefix paints six plain '#' glyphs followed by a space.
    let hash_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "#")
        .expect("h6 '######' prefix should render");
    let row = hash_idx / cols;
    let rstart = row * cols;
    let row_cells = &tl.layout[rstart..rstart + cols];
    let prefix: String = row_cells[0..7].iter().map(|b| b.word.as_str()).collect();
    assert_eq!(prefix, "###### ", "h6 must render the 6-level markdown prefix");
    let text: String = row_cells[7..12].iter().map(|b| b.word.as_str()).collect();
    assert_eq!(text, "Small", "heading text must follow the prefix on the same row");
}

#[test]
fn test_layout_hgroup_renders_heading_children() {
    init();
    // `<hgroup>` is a plain block container: its members (headings, and a
    // trailing <p>) keep their own block rendering. h4 defaults to 1 cell
    // per char so the canvas string is unambiguous.
    let html = "<hgroup><h4>Grouped</h4><p>tagline</p></hgroup>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(
        canvas.contains("#### Grouped"),
        "hgroup member heading must render with its own prefix"
    );
    assert!(
        canvas.contains("tagline"),
        "hgroup's trailing <p> child must render"
    );
}

#[test]
fn test_layout_address_italic_default() {
    init();
    // Browser UA default: unstyled `<address>` renders italic.
    let html = "<address>Contact</address>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "C")
        .expect("address text must render");
    let cell = &tl.layout[idx];
    assert!(
        cell.style.contains(FontStyle::ITALIC),
        "default <address> rendering must be italic (got style {:?})",
        cell.style
    );
}

#[test]
fn test_layout_address_font_style_normal_disables_italic() {
    init();
    // Author CSS resetting the UA default must take effect.
    let html = "<address>Contact</address>";
    let css = "address { font-style: normal; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "C")
        .expect("address text must render");
    let cell = &tl.layout[idx];
    assert!(
        !cell.style.contains(FontStyle::ITALIC),
        "`address {{ font-style: normal }}` must suppress the UA italic default (got {:?})",
        cell.style
    );
}

#[test]
fn test_layout_address_inherits_italic_from_ancestor() {
    init();
    // font-style is inherited: an ancestor rule must win over (and satisfy)
    // the UA default without an explicit rule on the address itself.
    let html = "<div><address>Contact</address></div>";
    let css = "div { font-style: italic; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "C")
        .expect("address text must render");
    let cell = &tl.layout[idx];
    assert!(
        cell.style.contains(FontStyle::ITALIC),
        "<address> must inherit font-style from its ancestor (got {:?})",
        cell.style
    );
}

// ============================================================
// <hr> + text-level semantics (strong/b/i/u/s/mark/…, code/kbd/samp, br)
// ============================================================

#[test]
fn test_layout_hr_renders_rule_across_width() {
    init();
    let html = "<hr>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "─")
        .expect("hr rule must render");
    let row = idx / cols;
    let rstart = row * cols;
    let rule_len = tl.layout[rstart..rstart + cols]
        .iter()
        .take_while(|b| b.word == "─")
        .count();
    assert_eq!(
        rule_len, cols,
        "hr must span the full content width (got {rule_len} cells)"
    );
}

#[test]
fn test_layout_strong_b_bold() {
    init();
    let html = "<p>alpha <strong>sbold</strong> <b>bravo</b></p>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let strong_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "s")
        .expect("strong text must render");
    let b_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "b")
        .expect("b text must render");
    let plain_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "a")
        .expect("plain text must render");
    assert!(
        tl.layout[strong_idx].style.contains(FontStyle::BOLD),
        "<strong> must be bold (got {:?})",
        tl.layout[strong_idx].style
    );
    assert!(
        tl.layout[b_idx].style.contains(FontStyle::BOLD),
        "<b> must be bold (got {:?})",
        tl.layout[b_idx].style
    );
    assert!(
        !tl.layout[plain_idx].style.contains(FontStyle::BOLD),
        "plain text must not be bold"
    );
}

#[test]
fn test_layout_text_semantics_styles() {
    init();
    // i → italic, u → underline, s → line-through, mark → yellow/black.
    // Each tag uses a distinct leading letter so the cell lookup is
    // unambiguous.
    let html = "<p><i>italic</i> <u>under</u> <s>strike</s> <mark>mark</mark></p>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let find = |word: &str| {
        tl.layout
            .iter()
            .position(|b| b.word == word)
            .unwrap_or_else(|| panic!("cell {word:?} must render"))
    };
    assert!(
        tl.layout[find("i")].style.contains(FontStyle::ITALIC),
        "<i> must be italic"
    );
    assert!(
        tl.layout[find("u")].style.contains(FontStyle::UNDERLINED),
        "<u> must be underlined"
    );
    assert!(
        tl.layout[find("s")].style.contains(FontStyle::CROSSED_OUT),
        "<s> must be line-through"
    );
    let mark_idx = find("m");
    assert_eq!(
        tl.layout[mark_idx].bcolor,
        Some(Color::Yellow),
        "<mark> must paint a yellow background"
    );
    assert_eq!(
        tl.layout[mark_idx].fcolor,
        Some(Color::Black),
        "<mark> must paint black foreground"
    );
}

#[test]
fn test_layout_edits_and_citation_styles() {
    init();
    // ins → underline, del → line-through, cite/dfn/var → italic.
    let html = "<p><ins>1</ins><del>2</del><cite>3</cite><dfn>4</dfn><var>5</var></p>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let find = |word: &str| {
        tl.layout
            .iter()
            .position(|b| b.word == word)
            .unwrap_or_else(|| panic!("cell {word:?} must render"))
    };
    assert!(
        tl.layout[find("1")].style.contains(FontStyle::UNDERLINED),
        "<ins> must be underlined"
    );
    assert!(
        tl.layout[find("2")].style.contains(FontStyle::CROSSED_OUT),
        "<del> must be line-through"
    );
    for word in ["3", "4", "5"] {
        assert!(
            tl.layout[find(word)].style.contains(FontStyle::ITALIC),
            "<cite>/<dfn>/<var> must be italic ({word})"
        );
    }
}

#[test]
fn test_layout_strong_font_weight_normal_disables_bold() {
    init();
    // Author CSS resetting the UA default must take effect.
    let html = "<p><strong>word</strong></p>";
    let css = "strong { font-weight: normal; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "w")
        .expect("strong text must render");
    assert!(
        !tl.layout[idx].style.contains(FontStyle::BOLD),
        "`strong {{ font-weight: normal }}` must suppress the UA bold (got {:?})",
        tl.layout[idx].style
    );
}

#[test]
fn test_layout_code_kbd_samp_render_plain() {
    init();
    // code / kbd / samp keep their text (UA monospace = the terminal's
    // native font, so no style flag applies).
    let html = "<p><code>fn main()</code> <kbd>ctrl</kbd> <samp>out</samp></p>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("fn main()"), "code text must render");
    assert!(canvas.contains("ctrl"), "kbd text must render");
    assert!(canvas.contains("out"), "samp text must render");
}

#[test]
fn test_layout_code_css_color_applies() {
    init();
    // The tag is addressable by CSS even though its UA style is a no-op in
    // the terminal.
    let html = "<p><code>x</code></p>";
    let css = "code { color: magenta; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let idx = tl
        .layout
        .iter()
        .position(|b| b.word == "x")
        .expect("code text must render");
    assert_eq!(
        tl.layout[idx].fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 255
        }),
        "code {{ color: magenta }} must paint the cell magenta"
    );
}

#[test]
fn test_layout_br_forces_line_break() {
    init();
    // `<br>` must force a line break even under the default `white-space:
    // normal` (where a plain newline would collapse).
    let html = "<p>aaa<br>bbb</p>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let a_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "a")
        .expect("pre-br text must render");
    let b_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "b")
        .expect("post-br text must render");
    assert!(
        b_idx / cols > a_idx / cols,
        "<br> must break the line"
    );
    assert_eq!(
        b_idx % cols,
        a_idx % cols,
        "post-br text must start at the same column"
    );
}

// ============================================================
// Canvas background (<html>/<body> propagate to the full viewport)
// ============================================================

#[test]
fn test_body_background_fills_canvas() {
    init();
    // Browser semantics: a short page still paints the whole terminal with
    // the <body> background (transparent <html> falls back to <body>).
    let html = "<body><h1>Hi</h1></body>";
    let css = "body { background-color: blue; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let rows = tl.layout.len() / cols;
    let last_row = &tl.layout[(rows - 1) * cols];
    assert_eq!(
        last_row.bcolor,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 255
        }),
        "body background must fill rows below the content"
    );
    assert_eq!(last_row.word, " ", "the fill must not add glyphs");
}

#[test]
fn test_html_background_wins_over_body_for_canvas() {
    init();
    // When both are set, the <html> background paints the canvas.
    let html = "<body><p>x</p></body>";
    let css = "html { background-color: green; } body { background-color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let rows = tl.layout.len() / cols;
    let cell = &tl.layout[(rows - 1) * cols];
    assert_eq!(
        cell.bcolor,
        Some(Color::Rgb {
            r: 0,
            g: 128,
            b: 0
        }),
        "<html> background must win over <body> for the canvas"
    );
}

#[test]
fn test_hsl_background_through_cascade_and_paint() {
    init();
    // A modern hsl() value must survive the cascade and paint the canvas via
    // the `background` shorthand alias.
    let html = "<body><p>x</p></body>";
    let css = "body { background: hsl(240, 100%, 50%); }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let rows = tl.layout.len() / cols;
    let cell = &tl.layout[(rows - 1) * cols];
    assert_eq!(
        cell.bcolor,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 255
        }),
        "hsl() through the `background` shorthand must paint the canvas"
    );
}

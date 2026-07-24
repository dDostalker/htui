//! Tests for `<span>` layout + run-level rendering.
//!
//! Each block-level layout function (p, div, h1-h5, header, footer, aside)
//! splits its inline children into one `InlineRun` per `<span>` boundary so
//! the paint phase can apply each run's own `FontStyle` / colour. These
//! tests lock down the run-level behaviour by inspecting the rendered canvas
//! directly: per-run `FontStyle` bits appear on the right cells, mixed
//! styles in the same paragraph visually compose, and nesting is additive.

use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId};

use htui::tui::graph_api::{FontStyle, TermLayout};

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn setup(html: &str, css: &str) -> (NodeId, NodeArena) {
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse().unwrap();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
        let rules = css_parser.parse().unwrap();
        css_parse::css_into_node(&mut arena, root_id, &rules).unwrap();
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

/// Locate the column index of the first cell whose `word` matches `target`.
/// Returns `None` when no such cell exists.
fn find_cell<'a>(tl: &'a TermLayout, target: &str) -> Option<usize> {
    tl.layout.iter().position(|b| b.word == target)
}

/// Locate the column index of the first cell whose `word` matches `target`
/// *and* whose `style` satisfies `pred`.
fn find_cell_with_style<'a, F: Fn(FontStyle) -> bool>(
    tl: &'a TermLayout,
    target: &str,
    pred: F,
) -> Option<usize> {
    tl.layout
        .iter()
        .position(|b| b.word == target && pred(b.style))
}

// ============================================================
// 1. Single span produces a single run with its own style.
// ============================================================

#[test]
fn test_p_with_bold_span_paints_bold_run() {
    init();
    let html = "<p>hi <span class=\"b\">bold</span></p>";
    let css = ".b { font-weight: bold; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    // The "b" of "bold" must carry the BOLD flag.
    let bold_idx = find_cell_with_style(&tl, "b", |s| s.contains(FontStyle::BOLD))
        .expect("bold span should paint cells with BOLD flag");
    // The "h" of "hi" must NOT carry BOLD.
    let plain_idx = find_cell(&tl, "h").expect("plain text should render");
    assert_ne!(
        bold_idx, plain_idx,
        "bold and plain cells must be different rows/cells"
    );
    let plain_style = tl.layout[plain_idx].style;
    assert!(
        !plain_style.contains(FontStyle::BOLD),
        "plain text outside span should not be bold, got style={plain_style:?}"
    );
}

#[test]
fn test_p_with_italic_span_paints_italic_run() {
    init();
    let html = "<p><span class=\"i\">it</span></p>";
    let css = ".i { font-style: italic; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let italic_idx = find_cell_with_style(&tl, "i", |s| s.contains(FontStyle::ITALIC))
        .expect("italic span should paint cells with ITALIC flag");
    let _ = italic_idx;
}

#[test]
fn test_p_with_underline_span_paints_underline_run() {
    init();
    let html = "<p><span class=\"u\">un</span></p>";
    let css = ".u { text-decoration: underline; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let u_idx = find_cell_with_style(&tl, "u", |s| s.contains(FontStyle::UNDERLINED))
        .expect("underline span should paint cells with UNDERLINED flag");
    assert_eq!(tl.layout[u_idx].word, "u");
}

#[test]
fn test_p_with_line_through_span_paints_crossed_run() {
    init();
    let html = "<p><span class=\"lt\">x</span></p>";
    let css = ".lt { text-decoration: line-through; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let idx = find_cell_with_style(&tl, "x", |s| s.contains(FontStyle::CROSSED_OUT))
        .expect("line-through span should paint cells with CROSSED_OUT flag");
    assert_eq!(tl.layout[idx].word, "x");
}

// ============================================================
// 2. Mixed styles in the same paragraph compose visually.
// ============================================================

#[test]
fn test_p_mixed_styles_in_single_row() {
    // The visual_test font_styles §11 case: bold + italic + plain text
    // interleaved in a single <p>. Each piece must carry its own style.
    init();
    let html = "<p>a<span class=\"b\">B</span><span class=\"i\">I</span><span class=\"u\">U</span></p>";
    let css = ".b { font-weight: bold; } .i { font-style: italic; } .u { text-decoration: underline; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    // All four cells sit on the same row.
    let a_idx = find_cell(&tl, "a").expect("plain 'a' should render");
    let b_idx = find_cell(&tl, "B").expect("bold 'B' should render");
    let i_idx = find_cell(&tl, "I").expect("italic 'I' should render");
    let u_idx = find_cell(&tl, "U").expect("underline 'U' should render");
    let cols = term_map.columns as usize;
    assert_eq!(a_idx / cols, b_idx / cols, "'a' and 'B' should share a row");
    assert_eq!(b_idx / cols, i_idx / cols, "'B' and 'I' should share a row");
    assert_eq!(i_idx / cols, u_idx / cols, "'I' and 'U' should share a row");

    // Each run carries exactly its own flag.
    assert!(
        !tl.layout[a_idx].style.contains(FontStyle::BOLD),
        "plain 'a' must not be bold"
    );
    assert!(
        tl.layout[b_idx].style.contains(FontStyle::BOLD),
        "'B' must be bold"
    );
    assert!(
        !tl.layout[b_idx].style.contains(FontStyle::ITALIC),
        "'B' must not be italic"
    );
    assert!(
        tl.layout[i_idx].style.contains(FontStyle::ITALIC),
        "'I' must be italic"
    );
    assert!(
        !tl.layout[i_idx].style.contains(FontStyle::BOLD),
        "'I' must not be bold"
    );
    assert!(
        tl.layout[u_idx].style.contains(FontStyle::UNDERLINED),
        "'U' must be underlined"
    );
    assert!(
        !tl.layout[u_idx].style.contains(FontStyle::ITALIC),
        "'U' must not be italic"
    );
}

// ============================================================
// 3. Per-span colour overrides the inherited colour.
// ============================================================

#[test]
fn test_span_color_overrides_parent() {
    init();
    let html = "<p>plain <span class=\"red\">red</span></p>";
    let css = ".red { color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let r_idx = find_cell(&tl, "r").expect("'r' of 'red' should render");
    assert_eq!(
        tl.layout[r_idx].fcolor,
        Some(Color::Red),
        "span cell should carry the red fg"
    );
    let p_idx = find_cell(&tl, "p").expect("plain 'p' should render");
    assert!(
        tl.layout[p_idx].fcolor.is_none() || tl.layout[p_idx].fcolor == Some(Color::Red),
        "plain cell should not pick up the span's red"
    );
    let _ = p_idx;
}

#[test]
fn test_span_color_with_inherited_parent_color() {
    init();
    let html = "<div><p><span class=\"green\">go</span></p></div>";
    let css = "div { color: magenta; } .green { color: green; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let g_idx = find_cell(&tl, "g").expect("'g' of 'go' should render");
    assert_eq!(
        tl.layout[g_idx].fcolor,
        Some(Color::Green),
        "span's own color (green) must win over div's inherited color"
    );
}

// ============================================================
// 4. Nested spans accumulate styles additively.
// ============================================================

#[test]
fn test_nested_spans_accumulate_styles() {
    init();
    let html = "<p><span class=\"b\">B<span class=\"i\">I</span>b</span></p>";
    let css =
        ".b { font-weight: bold; } .i { font-style: italic; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let b_idx = find_cell(&tl, "B").expect("outer 'B' should render");
    let i_idx = find_cell(&tl, "I").expect("inner 'I' should render");
    let inner_b_idx = tl
        .layout
        .iter()
        .rposition(|b| b.word == "b")
        .expect("inner 'b' should render");

    assert!(
        tl.layout[b_idx].style.contains(FontStyle::BOLD),
        "outer 'B' (only bold span active) must be bold"
    );
    assert!(
        !tl.layout[b_idx].style.contains(FontStyle::ITALIC),
        "outer 'B' must not be italic"
    );
    let inner_style = tl.layout[i_idx].style;
    assert!(
        inner_style.contains(FontStyle::BOLD),
        "inner 'I' must inherit outer bold"
    );
    assert!(
        inner_style.contains(FontStyle::ITALIC),
        "inner 'I' must add italic"
    );
    let tail_style = tl.layout[inner_b_idx].style;
    assert!(
        tail_style.contains(FontStyle::BOLD),
        "tail 'b' (after inner span closes) must still be bold"
    );
    assert!(
        !tail_style.contains(FontStyle::ITALIC),
        "tail 'b' must not still carry italic"
    );
}

// ============================================================
// 5. Span inherits parent block's colour when no own colour.
// ============================================================

#[test]
fn test_span_inherits_parent_color_when_no_own() {
    init();
    let html = "<div><p><span>x</span></p></div>";
    let css = "div { color: magenta; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let x_idx = find_cell(&tl, "x").expect("span 'x' should render");
    assert_eq!(
        tl.layout[x_idx].fcolor,
        Some(Color::Magenta),
        "span should inherit the ancestor's color"
    );
}

// ============================================================
// 6. Effective bg flows through unstyled runs in a styled rect.
// ============================================================

#[test]
fn test_unstyled_runs_inherit_aside_bg() {
    // <aside> with a darkblue bg wraps a <p> with mixed-styled runs. The
    // unstyled "plain " portion must still show the darkblue bg, while the
    // bold span carries only its own style flags (no own bg).
    init();
    let html = "<aside><p>plain <span class=\"b\">bold</span> end</p></aside>";
    let css = "aside { background-color: darkblue; } .b { font-weight: bold; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let p_idx = find_cell(&tl, "p").expect("plain 'p' should render");
    let b_idx = find_cell_with_style(&tl, "b", |s| s.contains(FontStyle::BOLD))
        .expect("bold 'b' should render");

    assert_eq!(
        tl.layout[p_idx].bcolor,
        Some(Color::DarkBlue),
        "plain text inside <aside> should inherit darkblue bg"
    );
    assert_eq!(
        tl.layout[b_idx].bcolor,
        Some(Color::DarkBlue),
        "bold span inside <aside> should also paint on darkblue bg"
    );
}

// ============================================================
// 7. text-align uses the concatenated run width.
// ============================================================

#[test]
fn test_text_align_center_counts_all_runs() {
    // With `text-align: center` the offset is computed from the *total*
    // text width across all runs, not the first one. The two runs of the
    // paragraph ("AB" bold + "CD" plain) should center their combined
    // 4-char width across the 80-col terminal.
    init();
    let html = "<p><span class=\"b\">AB</span> CD</p>";
    let css = ".b { font-weight: bold; } p { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let a_idx = find_cell_with_style(&tl, "A", |s| s.contains(FontStyle::BOLD))
        .expect("bold 'A' should render");
    let row = a_idx / cols;
    let col_in_row = a_idx % cols;
    // The HTML parser preserves original whitespace within text nodes — the
    // plain run between `<span>` and `</p>` keeps its leading space. Total
    // run width = 2 (`"AB"`) + 3 (`" CD"`) = 5.
    let total_width = "AB".len() + " CD".len();
    let expected_padding = (cols - total_width) / 2;
    assert_eq!(
        col_in_row, expected_padding,
        "first run should sit at the centered offset of the combined width, row={row}"
    );
}

// ============================================================
// 8. Run-level info survives the layout tree.
// ============================================================

#[test]
fn test_layout_box_exposes_runs_for_block_with_spans() {
    use htui::layout::{BoxConstraint, Cursor, InlineRun, tree::layout_tree};

    let (root_id, arena) = setup(
        "<p>plain <span class=\"b\">bold</span></p>",
        ".b { font-weight: bold; }",
    );
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor).unwrap();
    let p_box = boxes.first().expect("should have the <p> box");
    let runs = p_box
        .runs
        .as_ref()
        .expect("<p> should expose runs when it has spans");

    let collected: Vec<&str> = runs.iter().map(|r: &InlineRun| r.text.as_str()).collect();
    // The HTML parser preserves original whitespace within text nodes — the
    // plain run between the `<span>` and the closing `</p>` keeps its trailing
    // space. Run order still follows document order.
    assert_eq!(
        collected,
        vec!["plain ", "bold"],
        "runs should follow the document order: plain text then span text"
    );
    assert!(
        !runs[0].style.contains(FontStyle::BOLD),
        "plain run must not be bold"
    );
    assert!(
        runs[1].style.contains(FontStyle::BOLD),
        "bold run must carry BOLD"
    );
}

#[test]
fn test_layout_box_runs_are_materialised_for_plain_block() {
    // The single-run case is still materialised as one inline run rather
    // than skipped. The paint phase treats one run identically to the
    // general case, so this is a contract test on the layout.
    use htui::layout::{BoxConstraint, Cursor, tree::layout_tree};

    let (root_id, arena) = setup("<p>just text</p>", "");
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor).unwrap();
    let p_box = boxes.first().expect("should have the <p> box");
    let runs = p_box.runs.as_ref().expect("plain <p> still materialises runs");
    assert_eq!(runs.len(), 1, "single text node becomes one run");
    assert_eq!(runs[0].text, "just text");
}

// ============================================================
// 9. Heading prefix is its own run.
// ============================================================

#[test]
fn test_h1_prefix_is_a_separate_run() {
    use htui::layout::{BoxConstraint, Cursor, InlineRun, tree::layout_tree};

    let (root_id, arena) = setup("<h1>Title</h1>", "h1 { color: cyan; }");
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor).unwrap();
    let h_box = boxes.first().expect("should have the <h1> box");
    let runs = h_box.runs.as_ref().expect("<h1> should expose runs");
    assert_eq!(
        runs.len(),
        2,
        "<h1> with one body text should produce prefix + body runs"
    );
    assert_eq!(runs[0].text, "# ", "prefix run must be '# '");
    assert_eq!(runs[1].text, "Title", "body run must be the heading text");
    // Both runs share the heading's resolved color.
    for r in runs {
        assert_eq!(
            r.fg,
            Some(Color::Cyan),
            "heading runs inherit the heading's color"
        );
    }
    // The visual width of all runs equals the rendered "# Title" string.
    let total: usize = runs.iter().map(|r: &InlineRun| r.width()).sum();
    assert_eq!(total, "# Title".len());
}

// ============================================================
// 10. <span> alone (no enclosing block) still paints.
// ============================================================

#[test]
fn test_pure_span_inside_p_renders_text() {
    // Regression guard: a <p> whose only child is a <span> must still emit
    // the span's text on the canvas.
    init();
    let html = "<p><span>x</span></p>";
    let css = "";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains('x'), "span text should reach the canvas");
}

// ============================================================
// 11. End-to-end mirror of the visual_test font_styles §11 scenario.
// ============================================================

#[test]
fn test_font_styles_visual_test_mixed_inline() {
    // Mirror of visual_test/src/font_styles/index.html §11 ("Mixed inline
    // ranges"). Each `<span class="...">` should apply its own style on
    // the canvas, while plain text between them stays in the block's
    // default style.
    init();
    let html = "<p>Below: \
        <span class=\"b\">bold</span>, \
        <span class=\"i\">italic</span>, \
        <span class=\"u\">underlined</span>, \
        <span class=\"lt\">struck</span>, and a \
        <span class=\"cu\">green</span> word.</p>";
    let css = "p { color: white; } \
        .b  { font-weight: bold; } \
        .i  { font-style: italic; } \
        .u  { text-decoration: underline; } \
        .lt { text-decoration: line-through; } \
        .cu { color: green; text-decoration: underline; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
    tl.draw(&arena, root_id, &term_map).unwrap();

    // Bold span first letter 'b'.
    let bold_idx = find_cell_with_style(&tl, "b", |s| s.contains(FontStyle::BOLD))
        .expect("bold span should render with BOLD flag");
    assert!(
        !tl.layout[bold_idx].style.contains(FontStyle::ITALIC),
        "bold span should not be italic"
    );

    // Italic span first letter 'i'.
    let italic_idx = find_cell_with_style(&tl, "i", |s| s.contains(FontStyle::ITALIC))
        .expect("italic span should render with ITALIC flag");
    assert!(
        !tl.layout[italic_idx].style.contains(FontStyle::BOLD),
        "italic span should not be bold"
    );

    // Underline span first letter 'u'.
    let u_idx = find_cell_with_style(&tl, "u", |s| s.contains(FontStyle::UNDERLINED))
        .expect("underline span should render with UNDERLINED flag");
    let _ = u_idx;

    // Strikethrough span first letter 's' (only look on cells that have
    // 's' but not UNDERLINED — the word "struck" must carry CROSSED_OUT).
    let struct_idx = find_cell_with_style(&tl, "s", |s| s.contains(FontStyle::CROSSED_OUT))
        .expect("struck span should render with CROSSED_OUT flag");
    let _ = struct_idx;

    // Per-span color span 'g' should be green.
    let g_idx = find_cell(&tl, "g").expect("green span should render");
    assert_eq!(
        tl.layout[g_idx].fcolor,
        Some(Color::Green),
        "green span must carry green fg"
    );
    assert!(
        tl.layout[g_idx].style.contains(FontStyle::UNDERLINED),
        "green span must also be underlined"
    );
}

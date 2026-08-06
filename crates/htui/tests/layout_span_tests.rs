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
use htui::tdom::node::{NodeArena, NodeId, NodeType};

use htui::layout::LayoutBox;
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

/// Recursively walk the layout tree (depth-first) and return the first box
/// whose node type matches `target`. The top-level layout is rooted in the
/// transparent `<html>` wrapper, so tests that want to inspect a specific
/// element must descend through it instead of taking `boxes.first()`.
fn find_box_by_type<'a>(
    boxes: &'a [LayoutBox],
    target: &NodeType,
    arena: &NodeArena,
) -> Option<&'a LayoutBox> {
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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    find_cell_with_style(&tl, "i", |s| s.contains(FontStyle::ITALIC))
        .expect("italic span should paint cells with ITALIC flag");
}

#[test]
fn test_p_with_underline_span_paints_underline_run() {
    init();
    let html = "<p><span class=\"u\">un</span></p>";
    let css = ".u { text-decoration: underline; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

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
    let html =
        "<p>a<span class=\"b\">B</span><span class=\"i\">I</span><span class=\"u\">U</span></p>";
    let css =
        ".b { font-weight: bold; } .i { font-style: italic; } .u { text-decoration: underline; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let r_idx = find_cell(&tl, "r").expect("'r' of 'red' should render");
    assert_eq!(
        tl.layout[r_idx].fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 0
        }),
        "span cell should carry the red fg"
    );
    let p_idx = find_cell(&tl, "p").expect("plain 'p' should render");
    assert!(
        tl.layout[p_idx].fcolor.is_none()
            || tl.layout[p_idx].fcolor
                == Some(Color::Rgb {
                    r: 255,
                    g: 0,
                    b: 0
                }),
        "plain cell should not pick up the span's red"
    );
}

#[test]
fn test_span_color_with_inherited_parent_color() {
    init();
    let html = "<div><p><span class=\"green\">go</span></p></div>";
    let css = "div { color: magenta; } .green { color: green; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let g_idx = find_cell(&tl, "g").expect("'g' of 'go' should render");
    assert_eq!(
        tl.layout[g_idx].fcolor,
        Some(Color::Rgb {
            r: 0,
            g: 128,
            b: 0
        }),
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
    let css = ".b { font-weight: bold; } .i { font-style: italic; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let x_idx = find_cell(&tl, "x").expect("span 'x' should render");
    assert_eq!(
        tl.layout[x_idx].fcolor,
        Some(Color::Rgb {
            r: 255,
            g: 0,
            b: 255
        }),
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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let p_idx = find_cell(&tl, "p").expect("plain 'p' should render");
    let b_idx = find_cell_with_style(&tl, "b", |s| s.contains(FontStyle::BOLD))
        .expect("bold 'b' should render");

    assert_eq!(
        tl.layout[p_idx].bcolor,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 139
        }),
        "plain text inside <aside> should inherit darkblue bg"
    );
    assert_eq!(
        tl.layout[b_idx].bcolor,
        Some(Color::Rgb {
            r: 0,
            g: 0,
            b: 139
        }),
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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

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
    use htui::layout::{inner_formatting_context::flow::layout_tree, BoxConstraint, Cursor, };

    let (root_id, arena) = setup(
        "<p>plain <span class=\"b\">bold</span></p>",
        ".b { font-weight: bold; }",
    );
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor, 80, 24);
    let p_box = find_box_by_type(&boxes, &NodeType::P, &arena).expect("should have the <p> box");
    let runs = p_box
        .paint_style
        .runs
        .as_ref()
        .expect("<p> should expose runs when it has spans");
    // `paint_style.runs` is now `Vec<Vec<InlineRun>>` (one outer entry per
    // visual line after wrapping). At width 80, "plain " + "bold" both
    // fit on a single visual line, so the outer vec has one entry that
    // contains two runs in document order.
    assert_eq!(
        runs.len(),
        1,
        "plain + bold fits one visual line at width 80, got {} lines",
        runs.len()
    );
    let line_runs = &runs[0];
    let collected: Vec<&str> = line_runs.iter().map(|r| r.text.as_str()).collect();
    // The wrap layer splits the trailing space off the "plain " run
    // into its own `Space` token so the line-breaker can drop it on a
    // wrap point; the surviving runs read as `"plain"`, `" "`, `"bold"`
    // in document order.
    assert_eq!(
        collected,
        vec!["plain", " ", "bold"],
        "runs should follow the document order: plain text, then the space, then the span body"
    );
    assert!(
        !line_runs[0].style.contains(FontStyle::BOLD),
        "plain run must not be bold"
    );
    assert!(
        line_runs[2].style.contains(FontStyle::BOLD),
        "bold run must carry BOLD"
    );
}

#[test]
fn test_layout_box_runs_are_materialised_for_plain_block() {
    // The single-run case is still materialised as one inline run rather
    // than skipped. The paint phase treats one run identically to the
    // general case, so this is a contract test on the layout.
    use htui::layout::{inner_formatting_context::flow::layout_tree, BoxConstraint, Cursor};

    let (root_id, arena) = setup("<p>just text</p>", "");
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor, 80, 24);
    let p_box = find_box_by_type(&boxes, &NodeType::P, &arena).expect("should have the <p> box");
    let runs = p_box
        .paint_style
        .runs
        .as_ref()
        .expect("plain <p> still materialises runs");
    // `paint_style.runs` is `Vec<Vec<InlineRun>>` — at width 80, "just text"
    // fits on a single visual line, so the outer vec has one entry that
    // contains a single run.
    // `paint_style.runs` is `Vec<Vec<InlineRun>>` — at width 80, "just text"
    // fits on a single visual line. The wrap layer collapses the
    // whitespace inside the text node into a single space token, so the
    // outer vec has one entry with three runs: the leading word, the
    // space, and the trailing word.
    assert_eq!(runs.len(), 1, "single text node fits one visual line");
    assert_eq!(runs[0].len(), 3, "wrap splits words around the inner space");
    assert_eq!(runs[0][0].text, "just");
    assert_eq!(runs[0][1].text, " ");
    assert_eq!(runs[0][2].text, "text");
}

// ============================================================
// 9. Heading prefix is its own run.
// ============================================================

#[test]
fn test_h1_prefix_is_a_separate_run() {
    use htui::layout::{inner_formatting_context::flow::layout_tree, BoxConstraint, Cursor, };

    let (root_id, arena) = setup("<h1>Title</h1>", "h1 { color: cyan; }");
    let containing = BoxConstraint::root(&mock_term_map());
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor, 80, 24);
    let h_box = find_box_by_type(&boxes, &NodeType::H1, &arena).expect("should have the <h1> box");
    let runs = h_box.paint_style.runs.as_ref().expect("<h1> should expose runs");
    // `runs` is `Vec<Vec<InlineRun>>` — at width 80 the prefix and the
    // body fit on a single visual line, so the outer vec has one entry
    // that contains both runs in order: prefix first, then the heading
    // body.
    assert_eq!(
        runs.len(),
        1,
        "<h1> prefix + body fit one visual line at width 80"
    );
    let line_runs = &runs[0];
    // The wrap layer splits the prefix into `#` (the word) and ` `
    // (the space) before the body, so the visual line is built from
    // four runs in document order: prefix `#`, prefix space, body.
    assert_eq!(
        line_runs.len(),
        3,
        "<h1> prefix is split into '#' + ' ' before the body, total 3 runs"
    );
    assert_eq!(line_runs[0].text, "#", "first prefix chunk is '#'");
    assert_eq!(line_runs[1].text, " ", "second prefix chunk is the space");
    assert_eq!(line_runs[2].text, "Title", "body run must be the heading text");
    // All three runs share the heading's resolved color.
    for r in line_runs {
        assert_eq!(
            r.fg,
            Some(Color::Rgb {
                r: 0,
                g: 255,
                b: 255
            }),
            "heading runs inherit the heading's color"
        );
    }
    // The visual width of all runs equals the rendered "# Title" string.
    let total: usize = line_runs.iter().map(|r| r.width()).sum();
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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

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
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

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
        Some(Color::Rgb {
            r: 0,
            g: 128,
            b: 0
        }),
        "green span must carry green fg"
    );
    assert!(
        tl.layout[g_idx].style.contains(FontStyle::UNDERLINED),
        "green span must also be underlined"
    );
}

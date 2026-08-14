use crossterm::style::Color;
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;

use htui::tdom::node::{Node, NodeArena, NodeId, NodeType};
use htui::tui::graph_api::{TermLayout, char_width};

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
fn test_header_basic() {
    init();
    let (root_id, arena) = setup("<header>My Site</header>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("My Site"), "header should contain text");
}

#[test]
fn test_footer_basic() {
    init();
    let (root_id, arena) = setup("<footer>(c) 2024</footer>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("(c) 2024"), "footer should contain text");
}

#[test]
fn test_header_with_color() {
    init();
    let html = "<header>Colored</header>";
    let css = "header { color: blue; background-color: yellow; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let has_fg = tl
        .layout
        .iter()
        .any(|b| b.fcolor == Some(Color::Rgb { r: 0, g: 0, b: 255 }));
    let has_bg = tl.layout.iter().any(|b| {
        b.bcolor
            == Some(Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            })
    });
    assert!(has_fg, "should have blue foreground");
    assert!(has_bg, "should have yellow background");
}

#[test]
fn test_header_solid_bg() {
    init();
    let html = "<header>Solid</header>";
    let css = "header { background-color: green; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.word != " " && b.bcolor == Some(Color::Rgb { r: 0, g: 128, b: 0 }))
        .expect("should find green text");
    let content_row = content_idx / cols;
    let row_start = content_row * cols;
    let row_end = row_start + cols;
    let bg_count = tl.layout[row_start..row_end]
        .iter()
        .filter(|b| b.bcolor == Some(Color::Rgb { r: 0, g: 128, b: 0 }))
        .count();
    assert_eq!(bg_count, cols, "default 100% width should fill row");
}

#[test]
fn test_header_width_50pct() {
    // Browser-like geometry: a `<header>` with explicit `width: 50%` sits
    // flush left (cols 0..cols/2), NOT centered.
    init();
    let html = "<header>Half</header>";
    let css = "header { width: 50%; background-color: blue; color: white; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.bcolor.is_some() && b.word != " ")
        .expect("should find colored content");
    let content_row = content_idx / cols;
    let row = &tl.layout[content_row * cols..(content_row + 1) * cols];

    let block_start = 0;
    let block_end = cols / 2;
    let mut has_text = false;
    for (i, cell) in row.iter().enumerate().take(block_end).skip(block_start) {
        assert!(
            cell.bcolor.is_some(),
            "block should have bg col={i} (flush left)"
        );
        if cell.word != " " {
            has_text = true;
        }
    }
    assert!(has_text, "block should contain text");
    for cell in row.iter().take(cols).skip(block_end) {
        assert!(
            cell.bcolor.is_none(),
            "cols cols/2..cols should be bg-free (outside the block)"
        );
    }
}

#[test]
fn test_header_width_fixed() {
    // Browser-like geometry: a fixed-width `<header>` sits flush left at col 0.
    init();
    let html = "<header>Fix</header>";
    let css = "header { width: 20px; background-color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.bcolor.is_some() && b.word != " ")
        .expect("should find content");
    let row = content_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;

    let bg_cols: Vec<usize> = (rstart..rend)
        .filter(|&i| tl.layout[i].bcolor.is_some())
        .collect();
    assert_eq!(
        bg_cols.len(),
        20,
        "background should be exactly 20 columns (cols 0..20, flush left)"
    );
    // First bg cell must be at col 0 (flush left), not centered.
    let first_bg_col = bg_cols[0] - rstart;
    assert_eq!(
        first_bg_col, 0,
        "fixed-width header must sit flush left at col 0"
    );
}

#[test]
fn test_header_layout_structure() {
    init();
    let html = "<header>Site Title</header>";
    let css = "header { background-color: darkblue; text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(!canvas.contains('\u{2550}'));
    assert!(!canvas.contains('\u{2500}'));

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.bcolor.is_some() && b.word != " ")
        .expect("should find content");
    let row = content_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;
    for i in rstart..rend {
        assert!(
            tl.layout[i].bcolor.is_some(),
            "default width should fill entire row"
        );
    }
    let next = (row + 1) * cols;
    if next < tl.layout.len() {
        assert!(
            tl.layout[next].bcolor.is_none(),
            "next row should have no bg"
        );
    }
}

#[test]
fn test_footer_layout_structure() {
    init();
    let html = "<p>Above</p><footer>Copyright</footer>";
    let css = "footer { background-color: black; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let cols = term_map.columns as usize;

    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(!canvas.contains('\u{2500}'));
    assert!(!canvas.contains('\u{2550}'));

    let content_idx = tl
        .layout
        .iter()
        .position(|b| b.word == "C")
        .expect("should find footer");
    let row = content_idx / cols;
    let rstart = row * cols;
    let rend = rstart + cols;
    for i in rstart..rend {
        assert!(tl.layout[i].bcolor.is_some(), "footer row should have bg");
    }
}

#[test]
fn test_header_center_align() {
    init();
    let html = "<header>Centered</header>";
    let css = "header { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Centered"));
}

#[test]
fn test_header_right_align() {
    init();
    let html = "<header>Right</header>";
    let css = "header { text-align: right; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Right"));
}

#[test]
fn test_css_attr_text_align() {
    let (_, arena) = setup("<div></div>", "div { text-align: center; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(node.css_attr.get_text_align(), css_parse::TextAlign::Center);
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_bg_color() {
    let (_, arena) = setup("<div></div>", "div { background-color: red; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(
                node.css_attr.get_bg_color(),
                Some(Color::Rgb { r: 255, g: 0, b: 0 })
            );
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_bg_color_fallback() {
    let (_, arena) = setup("<div></div>", "div { background: green; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(
                node.css_attr.get_bg_color(),
                Some(Color::Rgb { r: 0, g: 128, b: 0 })
            );
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_font_weight() {
    let (_, arena) = setup("<div></div>", "div { font-weight: bold; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(node.css_attr.get_font_weight(), css_parse::FontWeight::Bold);
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_font_style_italic() {
    let (_, arena) = setup("<div></div>", "div { font-style: italic; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(
                node.css_attr.get_font_style_kind(),
                css_parse::FontStyleKind::Italic
            );
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_font_style_oblique() {
    let (_, arena) = setup("<div></div>", "div { font-style: oblique; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(
                node.css_attr.get_font_style_kind(),
                css_parse::FontStyleKind::Oblique
            );
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_text_decoration_underline() {
    let (_, arena) = setup("<div></div>", "div { text-decoration: underline; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(
                node.css_attr.get_text_decoration(),
                css_parse::TextDecoration::Underline
            );
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_text_decoration_line_through() {
    let (_, arena) = setup("<div></div>", "div { text-decoration: line-through; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(
                node.css_attr.get_text_decoration(),
                css_parse::TextDecoration::LineThrough
            );
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_css_attr_text_decoration_combined() {
    let (_, arena) = setup(
        "<div></div>",
        "div { text-decoration: underline line-through; }",
    );
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            assert_eq!(
                node.css_attr.get_text_decoration(),
                css_parse::TextDecoration::UnderlineLineThrough
            );
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_font_style_inherits_from_parent() {
    // font-style should propagate from parent to child like browsers do.
    let (_, arena) = setup("<div><span>x</span></div>", "div { font-style: italic; }");
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Span {
            assert_eq!(
                node.css_attr.get_font_style_kind(),
                css_parse::FontStyleKind::Italic,
                "child should inherit italic font-style from parent"
            );
            return;
        }
    }
    panic!("span not found");
}

#[test]
fn test_text_decoration_inherits_from_parent() {
    let (_, arena) = setup(
        "<div><span>x</span></div>",
        "div { text-decoration: underline; }",
    );
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Span {
            assert_eq!(
                node.css_attr.get_text_decoration(),
                css_parse::TextDecoration::Underline,
                "child should inherit underline text-decoration from parent"
            );
            return;
        }
    }
    panic!("span not found");
}

#[test]
fn test_block_style_aggregates_font_properties() {
    // Block-level aggregation: font-weight + font-style + text-decoration
    // should all be visible on a single cell.
    use htui::tui::graph_api::block_style;
    let (_, arena) = setup(
        "<div></div>",
        "div { font-weight: bold; font-style: italic; text-decoration: underline; }",
    );
    for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
        if node.node_type == NodeType::Div {
            let style = block_style(&node.css_attr);
            assert!(style.contains(htui::tui::graph_api::FontStyle::BOLD));
            assert!(style.contains(htui::tui::graph_api::FontStyle::ITALIC));
            assert!(style.contains(htui::tui::graph_api::FontStyle::UNDERLINED));
            assert!(!style.contains(htui::tui::graph_api::FontStyle::CROSSED_OUT));
            return;
        }
    }
    panic!("div not found");
}

#[test]
fn test_styled_space_cells_carry_style_flag() {
    // Regression: a styled paragraph like `<p>a b</p>` with underline must
    // populate the inter-letter space cell with the underline flag too —
    // otherwise `TermLayout::render`'s batch grouping prints the space
    // without SGR and the underline drops out between characters.
    use htui::tui::graph_api::FontStyle;
    let (root_id, arena) = setup("<p>a b</p>", "p { text-decoration: underline; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(
        canvas.contains("a b"),
        "expected to see `a b` in the canvas, got: {canvas:?}"
    );

    // The space cell must carry UNDERLINED so the renderer batches it with
    // the surrounding letters and applies the SGR.
    let styled_spaces = tl
        .layout
        .iter()
        .filter(|b| b.word == " " && b.style.contains(FontStyle::UNDERLINED))
        .count();
    assert!(
        styled_spaces >= 1,
        "expected at least one space cell to carry the underline flag, \
         got 0 in canvas: {canvas:?}"
    );
}

#[test]
fn test_bold_italic_underline_all_apply_to_text_run() {
    // The visual rendering path needs every style flag to show up on the
    // non-space glyphs as well. This guards against any future regression
    // where one of the flags gets dropped during aggregation or painting.
    use htui::tui::graph_api::FontStyle;
    let (root_id, arena) = setup(
        "<p>xyz</p>",
        "p { font-weight: bold; font-style: italic; text-decoration: underline line-through; }",
    );
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    for ch in ['x', 'y', 'z'] {
        let idx = tl
            .layout
            .iter()
            .position(|b| b.word == ch.to_string())
            .unwrap_or_else(|| panic!("should find character {ch}"));
        let style = tl.layout[idx].style;
        assert!(style.contains(FontStyle::BOLD), "{ch} should carry BOLD");
        assert!(
            style.contains(FontStyle::ITALIC),
            "{ch} should carry ITALIC"
        );
        assert!(
            style.contains(FontStyle::UNDERLINED),
            "{ch} should carry UNDERLINED"
        );
        assert!(
            style.contains(FontStyle::CROSSED_OUT),
            "{ch} should carry CROSSED_OUT"
        );
    }
}

#[test]
fn test_style_persists_after_consecutive_spaces() {
    // "a   b" (three spaces in the source). Under CSS `white-space: normal`
    // a run of whitespace collapses to a single space at the wrap layer,
    // so the rendered output is "a b" — but the surviving single space
    // must still carry the parent block's text-decoration so the
    // underline runs through the gap.
    use htui::tui::graph_api::FontStyle;
    let (root_id, arena) = setup("<p>a   b</p>", "p { text-decoration: underline; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);

    let styled_spaces = tl
        .layout
        .iter()
        .filter(|b| b.word == " " && b.style.contains(FontStyle::UNDERLINED))
        .count();
    // After white-space collapsing the wrap layer emits exactly one
    // space token between "a" and "b", and that token inherits the
    // surrounding text's style so it carries the underline flag.
    assert!(
        styled_spaces >= 1,
        "expected at least one space cell to carry underline, got {styled_spaces}"
    );
}

#[test]
fn test_render_emits_underline_sgr_across_spaces() {
    // End-to-end check: after rendering `<p>a b</p>` with underline, the
    // emitted ANSI byte stream must contain an underline SGR (`\x1B[4m`)
    // followed by the entire `a b` run (space included). Before the fix,
    // the renderer treated the space as "bare" and dropped the SGR.
    let (root_id, arena) = setup("<p>a b</p>", "p { text-decoration: underline; }");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    let bytes = tl.render_to_bytes();
    let rendered = String::from_utf8_lossy(&bytes);
    let underline_sgr = "\x1B[4m";

    assert!(
        rendered.contains(underline_sgr),
        "rendered output should contain underline SGR \\x1B[4m, got: {rendered:?}",
    );
    // The reset SGR (`\x1B[0m`) terminates a styled batch; ensure it
    // appears after the underlined run starts so the underline is active
    // for the whole batch including any spaces in it.
    if let Some(open_at) = rendered.find(underline_sgr) {
        let rest = &rendered[open_at..];
        let close_at = rest.find("\x1B[0m").unwrap_or_else(|| {
            panic!("styled batch must end with reset \\x1B[0m, got: {rendered:?}")
        });
        let batched_chars: String = rest[..close_at]
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        assert!(
            batched_chars.contains(' '),
            "underline batch should include the inter-letter space, got: {batched_chars:?}",
        );
    } else {
        panic!("underline SGR missing from rendered bytes: {rendered:?}");
    }
}

#[test]
fn test_is_rendered_as_bare_space_helper() {
    use crossterm::style::Color;
    use htui::tui::graph_api::{Block, FontStyle, is_rendered_as_bare_space};

    // Plain space, no bg, no style → bare.
    let bare = Block::new(" ".to_string(), None, None, FontStyle::empty());
    assert!(is_rendered_as_bare_space(&bare));

    // Space with underline → not bare (must keep SGR).
    let underlined = Block::new(" ".to_string(), None, None, FontStyle::UNDERLINED);
    assert!(!is_rendered_as_bare_space(&underlined));

    // Space with bg → not bare (bg must render).
    let with_bg = Block::new(" ".to_string(), None, Some(Color::Blue), FontStyle::empty());
    assert!(!is_rendered_as_bare_space(&with_bg));

    // Non-space character → never bare.
    let letter = Block::new("a".to_string(), None, None, FontStyle::empty());
    assert!(!is_rendered_as_bare_space(&letter));
}

#[test]
fn test_text_display_width() {
    assert_eq!(char_width('\u{2550}'), 1);
    assert_eq!(char_width('\u{2500}'), 1);
    assert_eq!(char_width('\u{4f60}'), 2);
}

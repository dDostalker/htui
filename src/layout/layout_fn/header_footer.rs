use anyhow::Result;

use crate::{
    html::css_parse::TextAlign,
    layout::{DEFAULT_LAYOUT, layout::TermMap},
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::{TermLayout, char_width},
};

fn text_display_width(s: &str) -> usize {
    s.chars().map(|c| char_width(c)).sum()
}

fn collect_text(arena: &NodeArena, node: &Node) -> String {
    let mut s = String::new();
    for child_id in node.child_ids() {
        if let NodeType::Text(t) = &arena[child_id].node_type {
            s.push_str(t);
        }
    }
    s
}

/// Render `<header>` — solid background block, width scaled by CSS `width`.
pub fn layout_header(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    arena: &NodeArena,
) -> Result<()> {
    if node.node_type != NodeType::Header {
        return Ok(());
    }
    if term_layout.is_full() {
        return Ok(());
    }
    render_block(node, term_layout, term_map, arena)
}

/// Render `<footer>` — same as header.
pub fn layout_footer(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    arena: &NodeArena,
) -> Result<()> {
    if node.node_type != NodeType::Footer {
        return Ok(());
    }
    if term_layout.is_full() {
        return Ok(());
    }
    render_block(node, term_layout, term_map, arena)
}

// ============================================================
// 核心：按 CSS `width` 渲染背景块
// ============================================================

fn render_block(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    arena: &NodeArena,
) -> Result<()> {
    let eff_cols = term_layout.effective_columns() as usize;
    let phys_cols = term_layout.columns as usize;
    let fg = node.css_attr.get_color();
    let bg = node.css_attr.get_bg_color();
    let align = node.css_attr.get_text_align();

    // CSS width 按有效列解析，再钳制到物理列
    let block_width = node.css_attr.get_width(eff_cols).unwrap_or(eff_cols);
    let block_width = block_width.min(phys_cols);
    // 在物理行内居中
    let left = (phys_cols.saturating_sub(block_width)) / 2;

    term_layout.new_line();

    if left > 0 {
        term_layout.paint_string(&" ".repeat(left), None, None);
    }
    render_text(
        node,
        term_layout,
        term_map,
        arena,
        fg,
        bg,
        align,
        block_width,
    );
    // 填充到物理行尾
    fill_to_col(term_layout, left + block_width, phys_cols, fg, bg);

    term_layout.new_line();
    term_layout.new_line();

    Ok(())
}

/// 渲染文本内容（在块宽度内对齐）
fn render_text(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    arena: &NodeArena,
    fg: Option<crossterm::style::Color>,
    bg: Option<crossterm::style::Color>,
    align: TextAlign,
    block_width: usize,
) {
    let text = collect_text(arena, node);
    let has_text = !text.is_empty();

    if has_text && align != TextAlign::Left {
        let text_width = text_display_width(&text);
        let padding = match align {
            TextAlign::Center => (block_width.saturating_sub(text_width)) / 2,
            TextAlign::Right => block_width.saturating_sub(text_width),
            TextAlign::Left => 0,
        };
        if padding > 0 {
            term_layout.paint_string(&" ".repeat(padding), fg, bg);
        }
    }

    for child_id in node.child_ids() {
        if term_layout.is_full() {
            break;
        }
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text(s) => {
                term_layout.paint_string(s, fg, bg);
            }
            _ => {
                for layout_fn in DEFAULT_LAYOUT {
                    let _ = layout_fn(child, term_layout, term_map, arena);
                }
            }
        }
    }
}

/// 从当前列填充到指定列（含背景色）
fn fill_to_col(
    term_layout: &mut TermLayout,
    target_col: usize,
    cols: usize,
    fg: Option<crossterm::style::Color>,
    bg: Option<crossterm::style::Color>,
) {
    let cur_col = term_layout.point as usize % cols;
    if cur_col < target_col {
        term_layout.paint_string(&" ".repeat(target_col - cur_col), fg, bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        html::{css_parse, html_parse},
        layout::layout::TermMap,
        tdom::node::Node,
        tui::graph_api::TermLayout,
    };
    use crossterm::style::Color;

    const COLS: u16 = 80;
    const ROWS: u16 = 24;

    fn mock_term_map() -> TermMap {
        TermMap {
            columns: COLS,
            rows: ROWS,
        }
    }

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn setup(html: &str, css: &str) -> (crate::tdom::node::NodeId, crate::tdom::node::NodeArena) {
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

    #[test]
    fn test_header_basic() {
        init();
        let (root_id, arena) = setup("<header>My Site</header>", "");
        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();
        let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
        assert!(canvas.contains("My Site"), "header 应包含文本");
    }

    #[test]
    fn test_footer_basic() {
        init();
        let (root_id, arena) = setup("<footer>(c) 2024</footer>", "");
        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();
        let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
        assert!(canvas.contains("(c) 2024"), "footer 应包含文本");
    }

    #[test]
    fn test_header_with_color() {
        init();
        let html = "<header>Colored</header>";
        let css = "header { color: blue; background-color: yellow; }";
        let (root_id, arena) = setup(html, css);
        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();
        let has_fg = tl.layout.iter().any(|b| b.fcolor == Some(Color::Blue));
        let has_bg = tl.layout.iter().any(|b| b.bcolor == Some(Color::Yellow));
        assert!(has_fg, "应有蓝色前景色");
        assert!(has_bg, "应有黄色背景色");
    }

    #[test]
    fn test_header_solid_bg() {
        init();
        let html = "<header>Solid</header>";
        let css = "header { background-color: green; }";
        let (root_id, arena) = setup(html, css);
        let term_map = mock_term_map();
        let cols = term_map.columns as usize;

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        let content_idx = tl
            .layout
            .iter()
            .position(|b| b.word != " " && b.bcolor == Some(Color::Green))
            .expect("应有绿色文本");
        let content_row = content_idx / cols;
        let row_start = content_row * cols;
        let row_end = row_start + cols;
        let bg_count = tl.layout[row_start..row_end]
            .iter()
            .filter(|b| b.bcolor == Some(Color::Green))
            .count();
        assert_eq!(bg_count, cols, "默认 100% 宽度时整行应有背景色");
    }

    /// width: 50% → 左右留白，中间实心块
    #[test]
    fn test_header_width_50pct() {
        init();
        let html = "<header>Half</header>";
        let css = "header { width: 50%; background-color: blue; color: white; }";
        let (root_id, arena) = setup(html, css);
        let term_map = mock_term_map();
        let cols = term_map.columns as usize;

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        // 内容行
        let content_idx = tl
            .layout
            .iter()
            .position(|b| b.bcolor.is_some() && b.word != " ")
            .expect("应有带背景色的内容");
        let content_row = content_idx / cols;
        let row = &tl.layout[content_row * cols..(content_row + 1) * cols];

        // 左侧留白无背景
        let left_margin = (cols - cols / 2) / 2;
        for i in 0..left_margin {
            assert!(row[i].bcolor.is_none(), "左留白应无背景 col={i}");
        }
        // 中间块有背景
        let block_end = left_margin + cols / 2;
        let mut has_text = false;
        for i in left_margin..block_end {
            assert!(row[i].bcolor.is_some(), "块内应有背景 col={i}");
            if row[i].word != " " {
                has_text = true;
            }
        }
        assert!(has_text, "块内应有文本");
        // 右侧留白无背景
        for i in block_end..cols {
            assert!(row[i].bcolor.is_none(), "右留白应无背景 col={i}");
        }
    }

    /// width: 20px → 固定 20 列宽
    #[test]
    fn test_header_width_fixed() {
        init();
        let html = "<header>Fix</header>";
        let css = "header { width: 20px; background-color: red; }";
        let (root_id, arena) = setup(html, css);
        let term_map = mock_term_map();
        let cols = term_map.columns as usize;

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        let content_idx = tl
            .layout
            .iter()
            .position(|b| b.bcolor.is_some() && b.word != " ")
            .expect("应有内容");
        let row = content_idx / cols;
        let rstart = row * cols;
        let rend = rstart + cols;

        // 背景格子应该恰好 20 列
        let bg_cols: Vec<usize> = (rstart..rend)
            .filter(|&i| tl.layout[i].bcolor.is_some())
            .collect();
        assert_eq!(bg_cols.len(), 20, "背景应正好 20 列");
    }

    /// 默认 100% → 无左右留白
    #[test]
    fn test_header_layout_structure() {
        init();
        let html = "<header>Site Title</header>";
        let css = "header { background-color: darkblue; text-align: center; }";
        let (root_id, arena) = setup(html, css);
        let term_map = mock_term_map();
        let cols = term_map.columns as usize;

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
        assert!(!canvas.contains('═'));
        assert!(!canvas.contains('─'));

        let content_idx = tl
            .layout
            .iter()
            .position(|b| b.bcolor.is_some() && b.word != " ")
            .expect("应有内容");
        let row = content_idx / cols;
        let rstart = row * cols;
        let rend = rstart + cols;
        for i in rstart..rend {
            assert!(tl.layout[i].bcolor.is_some(), "默认宽度整行应有背景");
        }
        // 下一行无背景
        let next = (row + 1) * cols;
        if next < tl.layout.len() {
            assert!(tl.layout[next].bcolor.is_none(), "下一行应无背景");
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

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
        assert!(!canvas.contains('─'));
        assert!(!canvas.contains('═'));

        let content_idx = tl
            .layout
            .iter()
            .position(|b| b.word == "C")
            .expect("应有 footer");
        let row = content_idx / cols;
        let rstart = row * cols;
        let rend = rstart + cols;
        for i in rstart..rend {
            assert!(tl.layout[i].bcolor.is_some(), "footer 整行应有背景");
        }
    }

    #[test]
    fn test_header_center_align() {
        init();
        let html = "<header>Centered</header>";
        let css = "header { text-align: center; }";
        let (root_id, arena) = setup(html, css);
        let term_map = mock_term_map();
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();
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
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();
        let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
        assert!(canvas.contains("Right"));
    }

    #[test]
    fn test_css_attr_text_align() {
        let (_, arena) = setup("<div></div>", "div { text-align: center; }");
        for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
            if node.node_type == crate::tdom::node::NodeType::Div {
                assert_eq!(node.css_attr.get_text_align(), css_parse::TextAlign::Center);
                return;
            }
        }
        panic!("未找到 div");
    }

    #[test]
    fn test_css_attr_bg_color() {
        let (_, arena) = setup("<div></div>", "div { background-color: red; }");
        for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
            if node.node_type == crate::tdom::node::NodeType::Div {
                assert_eq!(node.css_attr.get_bg_color(), Some(Color::Red));
                return;
            }
        }
        panic!("未找到 div");
    }

    #[test]
    fn test_css_attr_bg_color_fallback() {
        let (_, arena) = setup("<div></div>", "div { background: green; }");
        for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
            if node.node_type == crate::tdom::node::NodeType::Div {
                assert_eq!(node.css_attr.get_bg_color(), Some(Color::Green));
                return;
            }
        }
        panic!("未找到 div");
    }

    #[test]
    fn test_css_attr_font_weight() {
        let (_, arena) = setup("<div></div>", "div { font-weight: bold; }");
        for (_, node) in Node::descendants(&arena, arena.keys().next().unwrap()) {
            if node.node_type == crate::tdom::node::NodeType::Div {
                assert_eq!(node.css_attr.get_font_weight(), css_parse::FontWeight::Bold);
                return;
            }
        }
        panic!("未找到 div");
    }

    #[test]
    fn test_text_display_width() {
        assert_eq!(text_display_width("abc"), 3);
        assert_eq!(text_display_width("你好"), 4);
        assert_eq!(char_width('═'), 1);
        assert_eq!(char_width('─'), 1);
        assert_eq!(char_width('你'), 2);
    }
}

use anyhow::Result;

use crate::{
    layout::{DEFAULT_LAYOUT, layout::TermMap},
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::TermLayout,
};

/// 缩进前缀：两个空格 + 竖线 + 空格，模拟侧边栏样式
const ASIDE_INDENT: &str = "  │ ";

/// 渲染 `<aside>` — 缩进偏移块，左侧带竖线边框。
/// 不自行调用 leading new_line()，由前一个块级元素的 trailing new_line() 保证换行。
pub fn layout_aside(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    arena: &NodeArena,
) -> Result<()> {
    if node.node_type != NodeType::Aside {
        return Ok(());
    }
    if term_layout.is_full() {
        return Ok(());
    }

    let fg = node.css_attr.get_color();
    let bg = node.css_attr.get_bg_color();
    let cols = term_layout.columns as usize;

    // 如果当前不在行首，先跳到下一行
    if term_layout.point % term_layout.columns as u32 != 0 {
        fill_to_eol(term_layout, cols, fg, bg);
        term_layout.new_line();
    }

    // 绘制缩进前缀
    term_layout.paint_string(ASIDE_INDENT, fg, bg);

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
                    layout_fn(child, term_layout, term_map, arena)?;
                }
            }
        }
    }

    // 补齐当前行背景到行尾，并换行
    fill_to_eol(term_layout, cols, fg, bg);
    term_layout.new_line();

    Ok(())
}

/// 从当前列填充到行尾（仅在设置有背景色时生效）
fn fill_to_eol(
    term_layout: &mut TermLayout,
    cols: usize,
    fg: Option<crossterm::style::Color>,
    bg: Option<crossterm::style::Color>,
) {
    if bg.is_none() {
        return;
    }
    let cur_col = term_layout.point as usize % cols;
    if cur_col > 0 {
        let remaining = cols - cur_col;
        term_layout.paint_string(&" ".repeat(remaining), fg, bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        html::{css_parse, html_parse},
        layout::layout::TermMap,
        tui::graph_api::TermLayout,
    };
    use crossterm::style::Color;

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
    fn test_aside_basic() {
        init();
        let (root_id, arena) = setup("<aside>Side note</aside>", "");
        let term_map = TermMap::get_ter_map().unwrap();
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();
        let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
        assert!(canvas.contains(ASIDE_INDENT), "aside 应包含缩进竖线");
        assert!(canvas.contains("Side note"), "aside 应包含文本内容");
    }

    #[test]
    fn test_aside_with_color() {
        init();
        let html = "<aside>Colored aside</aside>";
        let css = "aside { color: blue; background-color: yellow; }";
        let (root_id, arena) = setup(html, css);
        let term_map = TermMap::get_ter_map().unwrap();
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();
        let has_fg = tl.layout.iter().any(|b| b.fcolor == Some(Color::Blue));
        let has_bg = tl.layout.iter().any(|b| b.bcolor == Some(Color::Yellow));
        assert!(has_fg, "应有蓝色前景色");
        assert!(has_bg, "应有黄色背景色");
    }

    #[test]
    fn test_aside_indent_present() {
        init();
        let (root_id, arena) = setup("<aside>Note</aside>", "");
        let term_map = TermMap::get_ter_map().unwrap();
        let cols = term_map.columns as usize;
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        // 找到带有竖线的内容行
        let bar_idx = tl.layout.iter()
            .position(|b| b.word == "│")
            .expect("应有竖线字符");
        let bar_row = bar_idx / cols;
        let row_start = bar_row * cols;

        // 竖线前应有 2 个空格缩进
        assert_eq!(tl.layout[row_start].word, " ", "缩进第一个字符应为空格");
        assert_eq!(tl.layout[row_start + 1].word, " ", "缩进第二个字符应为空格");
        assert_eq!(tl.layout[row_start + 2].word, "│", "第三个字符应为竖线");
        assert_eq!(tl.layout[row_start + 3].word, " ", "竖线后应为空格");
    }

    #[test]
    fn test_aside_with_nested_div() {
        init();
        let html = "<aside><div>Inner div</div></aside>";
        let (root_id, arena) = setup(html, "");
        let term_map = TermMap::get_ter_map().unwrap();
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();
        let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
        assert!(canvas.contains(ASIDE_INDENT), "aside 应包含缩进竖线");
        assert!(canvas.contains("Inner div"), "aside 应渲染嵌套 div 内容");
    }
}

use anyhow::Result;

use crate::{
    html::css_parse::TextAlign,
    layout::{DEFAULT_LAYOUT, layout::TermMap},
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::{TermLayout, char_width},
};

/// Calculate the display width of a string (ASCII = 1, CJK = 2).
#[doc(hidden)]
pub fn text_display_width(s: &str) -> usize {
    s.chars().map(|c| char_width(c)).sum()
}

/// Collect direct text children into a single string.
#[doc(hidden)]
pub fn collect_text(arena: &NodeArena, node: &Node) -> String {
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
                    if let Err(e) = layout_fn(child, term_layout, term_map, arena) {
                        log::debug!("Sub-layout skipped for {:?}: {}", child.node_type, e);
                    }
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

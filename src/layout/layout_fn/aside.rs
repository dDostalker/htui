use anyhow::Result;

use crate::{
    html::css_parse::TextAlign,
    layout::layout::TermMap,
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::{TermLayout, char_width},
};

/// Calculate the display width of a string (ASCII = 1, CJK = 2).
#[doc(hidden)]
pub fn text_display_width(s: &str) -> usize {
    s.chars().map(|c| char_width(c)).sum()
}

/// Collect all descendant text into a single string.
#[doc(hidden)]
pub fn collect_all_text(arena: &NodeArena, node: &Node) -> String {
    let mut s = String::new();
    for child_id in node.child_ids() {
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text(t) => s.push_str(t),
            _ => s.push_str(&collect_all_text(arena, child)),
        }
    }
    s
}

/// Render all descendant text with `fg`/`bg`, treating block children as line breaks.
fn render_aside_children(
    arena: &NodeArena,
    node: &Node,
    term_layout: &mut TermLayout,
    fg: Option<crossterm::style::Color>,
    bg: Option<crossterm::style::Color>,
) {
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
                // Block child: newline, then render its text with aside colors
                term_layout.new_line();
                render_aside_children(arena, child, term_layout, fg, bg);
            }
        }
    }
}

/// Render `<aside>` - block-level container with CSS `width` / `text-align`.
/// Background only covers text cells (inline-highlight), not filling the line.
pub fn layout_aside(
    node: &Node,
    term_layout: &mut TermLayout,
    _term_map: &TermMap,
    arena: &NodeArena,
) -> Result<()> {
    if node.node_type != NodeType::Aside {
        return Ok(());
    }
    if term_layout.is_full() {
        return Ok(());
    }

    let eff_cols = term_layout.effective_columns() as usize;
    let phys_cols = term_layout.columns as usize;
    let fg = node.css_attr.get_color();
    let bg = node.css_attr.get_bg_color();
    let align = node.css_attr.get_text_align();

    // CSS width: block is centered in terminal
    let block_width = node.css_attr.get_width(eff_cols).unwrap_or(eff_cols);
    let block_width = block_width.min(phys_cols);
    let left_margin = (phys_cols.saturating_sub(block_width)) / 2;

    // Jump to next line if not at line start (no background fill)
    if term_layout.point % phys_cols as u32 != 0 {
        term_layout.new_line();
    }

    // Left margin (no background)
    if left_margin > 0 {
        term_layout.paint_string(&" ".repeat(left_margin), None, None);
    }

    // text-align: measure all descendant text, add padding on first line
    let all_text = collect_all_text(arena, node);
    if !all_text.is_empty() && align != TextAlign::Left {
        let text_width = text_display_width(&all_text);
        let content_width = text_width.min(block_width);
        let padding = match align {
            TextAlign::Center => (block_width.saturating_sub(content_width)) / 2,
            TextAlign::Right => block_width.saturating_sub(content_width),
            TextAlign::Left => 0,
        };
        if padding > 0 {
            term_layout.paint_string(&" ".repeat(padding), None, None);
        }
    }

    // Render all descendant text with aside's fg/bg (block children = line breaks)
    render_aside_children(arena, node, term_layout, fg, bg);

    term_layout.new_line();
    Ok(())
}

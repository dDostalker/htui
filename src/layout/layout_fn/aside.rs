use anyhow::Result;

use crate::{
    html::css_parse::TextAlign,
    layout::layout::TermMap,
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::{TermLayout, char_width},
};

fn text_display_width(s: &str) -> usize {
    s.chars().map(|c| char_width(c)).sum()
}

fn collect_all_text(arena: &NodeArena, node: &Node) -> String {
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

#[cfg(test)]
mod tests {
    use super::text_display_width;
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
        assert!(canvas.contains("Side note"), "aside should contain text");
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
        assert!(has_fg, "should have blue foreground");
        assert!(has_bg, "should have yellow background");
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
        assert!(
            canvas.contains("Inner div"),
            "aside should render nested div"
        );
    }

    // ---- CSS width tests ----

    #[test]
    fn test_aside_width_50pct() {
        init();
        let html = "<aside>Half</aside>";
        let css = "aside { width: 50%; background-color: blue; color: white; }";
        let (root_id, arena) = setup(html, css);
        let term_map = TermMap::get_ter_map().unwrap();
        let cols = term_map.columns as usize;

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        // Background only on text cells (not filling whole block)
        let text_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
        assert_eq!(
            text_cells.len(),
            4,
            "bg should only cover text 'Half' (4 chars)"
        );
        for cell in &text_cells {
            assert_eq!(cell.bcolor, Some(Color::Blue));
            assert_eq!(cell.fcolor, Some(Color::White));
            assert_ne!(cell.word, " ");
        }

        // Text should be positioned after left margin
        let first_bg = tl.layout.iter().position(|b| b.bcolor.is_some()).unwrap();
        let col_in_row = first_bg % cols;
        let expected_left = (cols - cols / 2) / 2;
        assert!(
            col_in_row >= expected_left,
            "text should be within block, not in margin"
        );
    }

    #[test]
    fn test_aside_width_fixed_px() {
        init();
        let html = "<aside>Fix</aside>";
        let css = "aside { width: 20px; background-color: red; }";
        let (root_id, arena) = setup(html, css);
        let term_map = TermMap::get_ter_map().unwrap();

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        // Background only on text cells
        let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
        assert_eq!(
            bg_cells.len(),
            3,
            "bg should only cover text 'Fix' (3 chars)"
        );
        for cell in &bg_cells {
            assert_eq!(cell.bcolor, Some(Color::Red));
        }
    }

    #[test]
    fn test_aside_width_default_full() {
        init();
        let html = "<aside>Full width</aside>";
        let css = "aside { background-color: green; }";
        let (root_id, arena) = setup(html, css);
        let term_map = TermMap::get_ter_map().unwrap();

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        // Background only on text cells
        let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
        assert_eq!(
            bg_cells.len(),
            "Full width".len(),
            "bg should only cover text, not fill row"
        );
        for cell in &bg_cells {
            assert_eq!(cell.bcolor, Some(Color::Green));
        }
        // At least one non-space character has bg
        let text_with_bg = bg_cells.iter().filter(|c| c.word != " ").count();
        assert!(text_with_bg > 0, "text cells should have bg");
    }

    // ---- CSS text-align tests ----

    #[test]
    fn test_aside_text_align_center() {
        init();
        let html = "<aside>Centered</aside>";
        let css = "aside { text-align: center; width: 40px; background-color: darkblue; }";
        let (root_id, arena) = setup(html, css);
        let term_map = TermMap::get_ter_map().unwrap();
        let cols = term_map.columns as usize;

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        // Find the row with text
        let text_idx = tl
            .layout
            .iter()
            .position(|b| b.word != " ")
            .expect("should find text");
        let row = text_idx / cols;
        let rstart = row * cols;

        // Skip left margin, look within the 40px block
        let left_margin = (cols - 40) / 2;
        let text_start = rstart + left_margin;
        let block = &tl.layout[text_start..text_start + 40];

        // Text should be centered in block
        let first_non_space = block
            .iter()
            .position(|b| b.word != " ")
            .expect("should find text");
        let text_width = text_display_width("Centered");
        let expected_padding = (40 - text_width) / 2;
        assert_eq!(
            first_non_space, expected_padding,
            "text should be centered in block"
        );

        // Background only on text cells
        let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
        assert_eq!(bg_cells.len(), text_width, "bg only on text cells");
    }

    #[test]
    fn test_aside_text_align_right() {
        init();
        let html = "<aside>Right</aside>";
        let css = "aside { text-align: right; width: 40px; background-color: darkblue; }";
        let (root_id, arena) = setup(html, css);
        let term_map = TermMap::get_ter_map().unwrap();
        let cols = term_map.columns as usize;

        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root_id, &term_map).unwrap();

        let text_idx = tl
            .layout
            .iter()
            .position(|b| b.word != " ")
            .expect("should find text");
        let row = text_idx / cols;
        let rstart = row * cols;

        let left_margin = (cols - 40) / 2;
        let text_start = rstart + left_margin;
        let block = &tl.layout[text_start..text_start + 40];

        let first_non_space = block
            .iter()
            .position(|b| b.word != " ")
            .expect("should find text");
        let text_width = text_display_width("Right");
        let expected_padding = 40 - text_width;
        assert_eq!(
            first_non_space, expected_padding,
            "text should be right-aligned"
        );

        // Background only on text cells
        let bg_cells: Vec<_> = tl.layout.iter().filter(|b| b.bcolor.is_some()).collect();
        assert_eq!(bg_cells.len(), text_width, "bg only on text cells");
    }
}

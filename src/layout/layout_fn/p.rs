use anyhow::Result;

use crate::{
    layout::{DEFAULT_LAYOUT, layout::TermMap},
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::TermLayout,
};

pub fn layout_p(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    arena: &NodeArena,
) -> Result<()> {
    if node.node_type != NodeType::P {
        return Ok(());
    }
    if term_layout.is_full() {
        return Ok(());
    }
    term_layout.new_line();
    for child_id in node.child_ids() {
        if term_layout.is_full() {
            break;
        }
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text(s) => {
                let color = node.css_attr.get_color();
                term_layout.paint_string(s, color, None);
            }
            _ => {
                for layout_fn in DEFAULT_LAYOUT {
                    layout_fn(child, term_layout, term_map, arena)?;
                }
            }
        }
    }
    term_layout.new_line();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::html_parse::Parser;

    #[test]
    fn test_layout_p() {
        let mut p = Parser::new("<p>Hello</p><p>World</p>");
        let (root, arena, _) = p.parse().unwrap();
        let term_map = TermMap {
            columns: 80,
            rows: 24,
        };
        let mut tl = TermLayout::new(&term_map, 1.0).unwrap();
        tl.draw(&arena, root, &term_map).unwrap();
        let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
        assert!(canvas.contains("Hello"));
        assert!(canvas.contains("World"));
    }
}

use anyhow::Result;

use crate::{
    layout::{DEFAULT_LAYOUT, layout::TermMap},
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::TermLayout,
};

pub fn layout_hn(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    arena: &NodeArena,
) -> Result<()> {
    let n = match node.node_type {
        NodeType::H1 => 1, NodeType::H2 => 2, NodeType::H3 => 3,
        NodeType::H4 => 4, NodeType::H5 => 5, _ => return Ok(()),
    };
    term_layout.new_line();
    for child_id in node.child_ids() {
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text(s) => {
                let sharp = "#".repeat(n);
                let color = node.css_attr.get_color();
                term_layout.paint_string(&format!("{sharp} {s}"), color);
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

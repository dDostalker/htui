use anyhow::Result;

use crate::{
    TermMap,
    layout::DEFAULT_LAYOUT,
    tdom::node::{Node, NodeType},
    tui::graph_api::TermLayout,
};

pub fn layout_hn(tdom: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()> {
    let n = match tdom.node_type {
        NodeType::H1 => 1,
        NodeType::H2 => 2,
        NodeType::H3 => 3,
        NodeType::H4 => 4,
        NodeType::H5 => 5,
        _ => return Ok(()),
    };

    term_layout.new_line();

    for child in tdom.children() {
        match child.node_type.clone() {
            NodeType::Text(s) => {
                let sharp = "#".repeat(n);
                let s = format!("{sharp} {s}");
                let color = tdom.css_attr.get_color();
                term_layout.paint_string(&s, color);
            }
            _ => {
                for layout in DEFAULT_LAYOUT {
                    layout(child, term_layout, term_map)?;
                }
            }
        }
    }

    term_layout.new_line();
    Ok(())
}

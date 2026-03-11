use anyhow::Result;

use crate::{
    layout::{layout::TermMap, DEFAULT_LAYOUT},
    tdom::node::{Node, NodeType},
    tui::graph_api::TermLayout,
};

pub fn layout_div(tdom: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()> {
    if tdom.node_type != NodeType::Div {
        return Ok(());
    }

    term_layout.new_line();

    for child in tdom.children() {
        match child.node_type.clone() {
            NodeType::Text(s) => {
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

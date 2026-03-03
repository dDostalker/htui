use anyhow::Result;

use crate::{
    TermMap,
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
    for child in tdom.children() {
        match child.node_type.clone() {
            NodeType::Text(s) => {
                let sharp = "#".repeat(n);
                let s = format!("{sharp} {s}");
                let column = term_map.columns;
                let color = tdom.css_attr.get_color();
                term_layout.paint_string(&s, color);
                let point_val = term_layout.point;
                let yu = column as u64 - point_val % column as u64;
                let point = &mut term_layout.point;
                *point += yu;
            }
            _ => {}
        }
    }
    Ok(())
}

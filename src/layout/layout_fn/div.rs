use anyhow::Result;

use crate::{
    layout::layout::TermMap,
    tdom::node::{Node, NodeType},
    tui::graph_api::TermLayout,
};

//仿照p.rs的内容，div需要换行，不确定有没有用，先不加入到DEFAULT_LAYOUT
pub fn layout_div(tdom: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()> {
    if tdom.node_type != NodeType::Div {
        return Ok(());
    }
    for child in tdom.children() {
        match child.node_type.clone() {
            NodeType::Text(s) => {
                let column = term_map.columns;
                let color = tdom.css_attr.get_color();
                term_layout.paint_string(&s, color);
                let point_val = term_layout.point;
                let yu = column as u64 - point_val % column as u64;
                let point = &mut term_layout.point;
                *point += yu; // 换行
            }
            _ => {}
        }
    }
    Ok(())
}

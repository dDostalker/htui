use anyhow::Result;

use crate::{
    layout::layout::TermMap,
    tdom::node::{Node, NodeType},
    tui::graph_api::TermLayout,
};

//仿照p.rs的内容，但是span不换行所以去掉了相关部分，不确定有没有用，先不加入到DEFAULT_LAYOUT
pub fn layout_span(tdom: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()> {
    if tdom.node_type != NodeType::Span {
        return Ok(());
    }
    for child in tdom.children() {
        match child.node_type.clone() {
            NodeType::Text(s) => {
                let color = tdom.css_attr.get_color();
                term_layout.paint_string(&s, color); // 不换行
            }
            _ => {}
        }
    }
    Ok(())
}

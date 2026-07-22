use anyhow::Result;

use crate::{
    layout::layout::TermMap,
    tdom::node::{Node, NodeArena, NodeType},
    tui::graph_api::TermLayout,
};

pub fn layout_img(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    _arena: &NodeArena,
) -> Result<()> {
    if node.node_type != NodeType::Img { return Ok(()); }
    let src = match node.attributes.get_src() { Some(s) => s.clone(), None => return Ok(()), };
    let max_w = node.attributes.get_width().unwrap_or(term_map.columns as u32 * 2);
    let max_h = node.attributes.get_height().unwrap_or(term_map.rows as u32 * 4);
    term_layout.new_line();
    term_layout.paint_img(src, (max_w, max_h));
    term_layout.new_line();
    Ok(())
}

use anyhow::Result;

use crate::{
    layout::layout::TermMap,
    tdom::node::{Node, NodeType},
    tui::graph_api::TermLayout,
};

pub fn layout_img(tdom: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()> {
    if tdom.node_type != NodeType::Img {
        return Ok(());
    }

    let src = match tdom.attributes.get_src() {
        Some(s) => s.clone(),
        None => return Ok(()),
    };

    // width/height 属性为像素尺寸，未指定时用终端字符数折算回像素
    // 每个盲文字符对应 2×4 像素，所以终端列数 × 2 = 像素宽，行数 × 4 = 像素高
    let max_w = tdom
        .attributes
        .get_width()
        .unwrap_or(term_map.columns as u32 * 2);
    let max_h = tdom
        .attributes
        .get_height()
        .unwrap_or(term_map.rows as u32 * 4);

    term_layout.new_line();

    term_layout.paint_img(src, (max_w, max_h));

    term_layout.new_line();
    Ok(())
}

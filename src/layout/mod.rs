use anyhow::Result;

use crate::{
    layout::{
        layout::TermMap,
        layout_fn::{div::layout_div, head::layout_hn, img::layout_img, p::layout_p},
    },
    tdom::node::Node,
    tui::graph_api::TermLayout,
};

pub mod layout;
pub mod layout_fn;

type LayoutFn = fn(node: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()>;

/// 这里的循环需要有固定的顺序
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[layout_p, layout_hn, layout_img, layout_div];

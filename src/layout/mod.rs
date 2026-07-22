use anyhow::Result;

use crate::{
    layout::{
        layout::TermMap,
        layout_fn::{div::layout_div, head::layout_hn, img::layout_img, p::layout_p},
    },
    tdom::node::{Node, NodeArena, NodeId},
    tui::graph_api::TermLayout,
};

pub mod layout;
pub mod layout_fn;

pub type LayoutFn =
    fn(node: &Node, term_layout: &mut TermLayout, term_map: &TermMap, arena: &NodeArena) -> Result<()>;

/// 布局函数注册表（按优先级排列）
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[layout_p, layout_hn, layout_img, layout_div];

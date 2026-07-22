use anyhow::Result;

use crate::{
    layout::{
        layout::TermMap,
        layout_fn::{
            aside::layout_aside, div::layout_div, head::layout_hn, header_footer::layout_footer,
            header_footer::layout_header, img::layout_img, p::layout_p,
        },
    },
    tdom::node::{Node, NodeArena},
    tui::graph_api::TermLayout,
};

pub mod layout;
pub mod layout_fn;

pub type LayoutFn = fn(
    node: &Node,
    term_layout: &mut TermLayout,
    term_map: &TermMap,
    arena: &NodeArena,
) -> Result<()>;

/// Layout function registry (ordered by priority).
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[
    layout_p,
    layout_hn,
    layout_img,
    layout_div,
    layout_header,
    layout_footer,
    layout_aside,
];

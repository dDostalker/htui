use crate::{
    layout::layout::TermMap,
    tdom::node::{NodeArena, NodeId},
};

pub mod block;
pub mod box_tree;
pub mod flex;
pub mod grid;
pub mod layout;
pub mod paint;
pub mod tree;

pub use crate::html::box_edges::{Border, BorderSides, BorderStyle, BoxEdges, EdgeSizes};
pub use box_tree::{BoxKind, Cursor, InlineRun, LayoutBox, LayoutMode, Rect, new_flex_item};
pub use tree::FontContext;

/// The containing block available to a layout function.
///
/// A child element resolves its own width / horizontal position relative to
/// its containing block, mirroring CSS containing-block semantics.
#[derive(Debug, Clone, Copy)]
pub struct BoxConstraint {
    pub x: usize,
    pub width: usize,
}

impl BoxConstraint {
    /// Creates a root constraint spanning the terminal width.
    pub fn root(term_map: &TermMap) -> Self {
        Self {
            x: 0,
            width: term_map.columns as usize,
        }
    }
}

/// Layout function signature for the two-phase pipeline.
///
/// Each registered function inspects `node_id`; returns `Ok(None)` to skip
/// (passes through to the next entry), or `Ok(Some(LayoutBox))` to claim the
/// node. The builder in [`tree::layout_tree`] walks the registry in order and
/// stops at the first claimer. `font_ctx` carries the parent's effective
/// font-size (and the root font-size) so `em` / `rem` lengths resolve against
/// the active font-size context.
pub type LayoutFn = fn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox>;

/// Layout function registry.
///
/// Each element type that needs custom layout declares a function here. The
/// dispatcher iterates this list and uses the first non-`None` result.
///
/// The first entry is the flex dispatcher: it claims every block element
/// that opts into `display: flex`, regardless of its tag. For non-flex
/// nodes it returns `None` and the registry falls through to the
/// element-specific block layout, so flex and the existing block
/// layout are mutually exclusive without duplicating code.
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[
    flex::flex::layout_flex_dispatch,
    block::aside::layout_aside,
    block::body::layout_body,
    block::div::layout_div,
    block::head::layout_hn,
    block::head::layout_title,
    block::head::layout_meta,
    block::header_footer::layout_footer,
    block::header_footer::layout_header,
    block::html::layout_html,
    block::img::layout_img,
    block::list::layout_li,
    block::list::layout_ol,
    block::list::layout_ul,
    block::p::layout_p,
    block::table::layout_row_group,
    block::table::layout_table,
    block::table::layout_td,
    block::table::layout_th,
    block::table::layout_tr,
];

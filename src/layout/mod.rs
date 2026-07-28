use crate::{
    layout::layout::TermMap,
    tdom::node::{NodeArena, NodeId},
};

pub mod box_tree;
pub mod layout;
pub mod layout_fn;
pub mod paint;
pub mod tree;

pub use crate::html::box_edges::{Border, BorderSides, BorderStyle, BoxEdges, EdgeSizes};
pub use box_tree::{BoxKind, Cursor, InlineRun, LayoutBox, Rect};
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
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[
    layout_fn::p::layout_p,
    layout_fn::head::layout_hn,
    layout_fn::img::layout_img,
    layout_fn::div::layout_div,
    layout_fn::html::layout_html,
    layout_fn::body::layout_body,
    layout_fn::head::layout_title,
    layout_fn::head::layout_meta,
    layout_fn::header_footer::layout_header,
    layout_fn::header_footer::layout_footer,
    layout_fn::aside::layout_aside,
    layout_fn::list::layout_li,
    layout_fn::list::layout_ol,
    layout_fn::list::layout_ul,
    layout_fn::table::layout_table,
    layout_fn::table::layout_tr,
    layout_fn::table::layout_th,
    layout_fn::table::layout_td,
    layout_fn::table::layout_row_group,
];

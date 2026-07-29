//! `<tr>` is a row wrapper. Column measurement and final rect assignment
//! are owned by `layout_table`; the row itself produces no layout box.

use crate::{
    layout::{BoxConstraint, Cursor, FontContext, LayoutBox, Rect},
    tdom::node::{NodeArena, NodeId},
};

/// Pass-through layout function for `<tr>`.
pub fn layout_tr(
    _arena: &NodeArena,
    _node_id: NodeId,
    _containing: BoxConstraint,
    _cursor: &mut Cursor,
    _font_ctx: FontContext,
    _pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    None
}

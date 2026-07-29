//! `<thead>` / `<tbody>` / `<tfoot>` are row-grouping wrappers; they don't
//! produce a layout box of their own. The table's `collect_rows` flattens
//! through them, so the registry simply passes them. We register them so
//! future changes (e.g. sticky `<thead>`) have a natural hook without
//! re-plumbing the registry.

use crate::{
    layout::{BoxConstraint, Cursor, FontContext, Rect},
    tdom::node::{NodeArena, NodeId},
};

/// Pass-through layout function for `<thead>` / `<tbody>` / `<tfoot>`.
pub fn layout_row_group(
    _arena: &NodeArena,
    _node_id: NodeId,
    _containing: BoxConstraint,
    _cursor: &mut Cursor,
    _font_ctx: FontContext,
    _pre_allocated_rect: Option<Rect>,
) -> Option<crate::layout::LayoutBox> {
    None
}

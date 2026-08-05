//! `<td>` / `<th>` pass-through functions.
//!
//! Cell layout is owned by `layout_table`'s `layout_cell` helper; these
//! entry points exist so the registry's `dispatch` doesn't fall through to
//! the generic block handler. `<th>` is currently identical to `<td>` —
//! kept as a separate function so a future change (e.g. default bold +
//! center) has a natural hook.

use crate::{
    layout::{BoxConstraint, Cursor, LayoutBox, LayoutContext, Rect},
    tdom::node::{NodeArena, NodeId},
};

/// Pass-through layout function for `<td>`.
pub fn layout_td(
    _arena: &NodeArena,
    _node_id: NodeId,
    _containing: BoxConstraint,
    _cursor: &mut Cursor,
    _context: LayoutContext,
    _pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    None
}

/// Pass-through layout function for `<th>`.
pub fn layout_th(
    _arena: &NodeArena,
    _node_id: NodeId,
    _containing: BoxConstraint,
    _cursor: &mut Cursor,
    _context: LayoutContext,
    _pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    None
}

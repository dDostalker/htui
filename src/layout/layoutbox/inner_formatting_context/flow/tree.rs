//! Block formatting context — tree builder.
//!
//! Walks the DOM through the [`DEFAULT_LAYOUT`](crate::layout::DEFAULT_LAYOUT)
//! registry and assembles a forest of [`LayoutBox`]es rooted at the document.
//! Block-level children of the document share a single vertical [`Cursor`],
//! so each block claims a deterministic row range.
//!
//! This module is the block counterpart of the flex dispatch in
//! [`crate::layout::layoutbox::inner_formatting_context::flex`].
//! Both register into the same layout registry; the flex dispatch claims
//! flex containers first, and non-flex nodes fall through to the per-element
//! block handlers declared in this module's siblings.

use crate::{
    html::css_parse::FontContext,
    layout::{BoxConstraint, Cursor, DEFAULT_LAYOUT, LayoutBox, Rect},
    tdom::node::{NodeArena, NodeId},
};



/// Build a layout forest for `root_id`'s block children.
///
/// `containing` is the root containing block (typically the full terminal
/// width). `cursor` tracks the next free row and is advanced in-place so
/// callers can chain multiple passes. `viewport_cols` / `viewport_rows`
/// supply the viewport size for `vw` / `vh` resolution.
pub fn layout_tree(
    arena: &NodeArena,
    root_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    viewport_cols: usize,
    viewport_rows: usize,
) -> Vec<LayoutBox> {
    let children: Vec<NodeId> = arena[root_id].children.clone();
    layout_children(
        arena,
        &children,
        containing,
        cursor,
        FontContext::root(viewport_cols, viewport_rows),
    )
}

/// Lay out a sequence of sibling node IDs under `containing`.
pub fn layout_children(
    arena: &NodeArena,
    children: &[NodeId],
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Vec<LayoutBox> {
    let mut boxes = Vec::new();
    for &child_id in children {
        if let Some(b) = dispatch(arena, child_id, containing, cursor, font_ctx, None) {
            boxes.push(b);
        }
    }
    boxes
}

/// Iterate the layout registry; first claim wins.
///
/// `pre_allocated_rect` is forwarded to every layout function so the flex
/// layout can hand each flex item a fixed rectangle. Public so the flex
/// layout can recurse into the dispatcher after deciding each item's rect.
pub fn dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    for layout_fn in DEFAULT_LAYOUT {
        if let Some(b) = layout_fn(arena, node_id, containing, cursor, font_ctx, pre_allocated_rect) {
            return Some(b);
        }
    }
    log::debug!("No layout handler for {:?}", arena[node_id].node_type);
    None
}
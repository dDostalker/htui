//! Layout tree construction.
//!
//! Walks the DOM through the [`DEFAULT_LAYOUT`](crate::layout::DEFAULT_LAYOUT)
//! registry, building a forest of [`LayoutBox`]es rooted at the document.
//! Block-level children of the document share a single vertical [`Cursor`],
//! so each block claims a deterministic row range.

use anyhow::Result;

use crate::{
    layout::{BoxConstraint, Cursor, DEFAULT_LAYOUT, LayoutBox},
    tdom::node::{Node, NodeArena, NodeId},
};

/// Build a layout forest for `root_id`'s block children.
///
/// `containing` is the root containing block (typically the full terminal
/// width). `cursor` tracks the next free row and is advanced in-place so
/// callers can chain multiple passes.
pub fn layout_tree(
    arena: &NodeArena,
    root_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Vec<LayoutBox>> {
    let children: Vec<NodeId> = arena[root_id].children.clone();
    layout_children(arena, &children, containing, cursor)
}

/// Lay out a sequence of sibling node IDs under `containing`.
pub fn layout_children(
    arena: &NodeArena,
    children: &[NodeId],
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Vec<LayoutBox>> {
    let mut boxes = Vec::new();
    for &child_id in children {
        if let Some(b) = dispatch(arena, child_id, containing, cursor)? {
            boxes.push(b);
        }
    }
    Ok(boxes)
}

/// Iterate the layout registry; first claim wins.
fn dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    for layout_fn in DEFAULT_LAYOUT {
        if let Some(b) = layout_fn(arena, node_id, containing, cursor)? {
            return Ok(Some(b));
        }
    }
    log::debug!("No layout handler for {:?}", arena[node_id].node_type);
    Ok(None)
}

/// Collect text from direct text descendants, in document order.
///
/// Used by block elements to fold `<span>foo</span><span>bar</span>` into a
/// single inline run for the common simple case. Does **not** recurse into
/// nested block children.
pub fn collect_direct_text(arena: &NodeArena, node: &Node) -> String {
    let mut out = String::new();
    for child_id in node.child_ids() {
        let child = &arena[child_id];
        match &child.node_type {
            crate::tdom::node::NodeType::Text(s) => out.push_str(s),
            crate::tdom::node::NodeType::Span => {
                out.push_str(&collect_direct_text(arena, child));
            }
            _ => {}
        }
    }
    out
}

/// Resolve a block-level element's CSS width into a column count, clamped to
/// the containing block.
pub fn resolve_block_width(css_width: Option<usize>, containing_width: usize) -> usize {
    css_width.unwrap_or(containing_width).min(containing_width)
}

/// Compute the centered left offset for a block of `block_width` inside a
/// containing block of `containing_width`, starting at `containing.x`.
pub fn centered_left(containing: BoxConstraint, block_width: usize) -> usize {
    containing.x + containing.width.saturating_sub(block_width) / 2
}


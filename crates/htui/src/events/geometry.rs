//! Immutable layout snapshot shared between the render thread and the
//! hit-test thread (region collection).

use crate::{
    layout::layoutbox::LayoutBox,
    tdom::node::{NodeArena, NodeId},
};

/// Immutable snapshot of one layout pass.
///
/// Owns the DOM arena, the laid-out box forest and the root node id. Built by
/// the render thread after each `Redraw` and shared through `Arc`; both
/// consumers treat it as read-only, so no locking is needed past construction.
pub struct Geometry {
    pub arena: NodeArena,
    pub boxes: Vec<LayoutBox>,
    /// Root node id of this layout session (needed to re-run the CSS cascade
    /// for `:hover` re-styling).
    pub root_id: NodeId,
}

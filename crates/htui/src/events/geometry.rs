//! Immutable layout snapshot shared between the render thread and the
//! hit-test thread (region collection).

use std::sync::Arc;

use crate::{
    layout::layoutbox::LayoutBox,
    tdom::node::{NodeId, SharedNodeArena},
};

/// Immutable snapshot of one layout pass.
///
/// Holds the DOM arena, the laid-out box forest and the root node id. Built by
/// the render thread after each `Redraw` and shared through `Arc`; both
/// consumers treat it as read-only, so no locking is needed past construction.
/// The arena itself is `Arc`-shared so the render thread's document state and
/// the input loop's read snapshot reference the same allocation instead of
/// cloning the tree per frame.
pub struct Geometry {
    pub arena: SharedNodeArena,
    pub boxes: Vec<LayoutBox>,
    /// Root node id of this layout session (needed to re-run the CSS cascade
    /// for `:hover` re-styling).
    pub root_id: NodeId,
}

/// A layout snapshot shared from the render thread to the hit-test thread.
pub type SharedGeometry = Arc<Geometry>;

//! Flex formatting context implementation.
//!
//! CSS flexbox for block-level elements. The entry point is
//! [`tree::layout_flex_dispatch`], registered first in
//! [`crate::layout::DEFAULT_LAYOUT`] so it preempts the element-specific
//! block layout whenever the node has `display: flex`. For non-flex nodes
//! it returns `None`, letting the registry fall through to the next claimer.
//!
//! This module is the flex counterpart of [`super::flow`]. Both register
//! into the same layout registry; the flex dispatch claims flex containers
//! first, and non-flex nodes fall through to the per-element block handlers
//! in [`super::flow`].

mod tree;

pub use tree::layout_flex_dispatch;

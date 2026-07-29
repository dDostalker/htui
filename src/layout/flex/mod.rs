//! Flex layout.
//!
//! CSS flexbox for block-level elements. The entry point is
//! [`flex::layout_flex_dispatch`], registered first in
//! [`crate::layout::DEFAULT_LAYOUT`] so it preempts the element-specific
//! block layout whenever the node has `display: flex`. For non-flex nodes
//! it returns `None`, letting the registry fall through to the next
//! claimer.

pub mod flex;

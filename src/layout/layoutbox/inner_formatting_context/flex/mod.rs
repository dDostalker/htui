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
//!
//! ## Module layout
//!
//! ```text
//! flex/
//! ├── mod.rs   ← FlexExt + impl FormattingContext::Flex (this file)
//! └── tree.rs  ← layout_flex_dispatch + flex algorithm
//! ```

use super::FormattingContext;

mod tree;

pub use tree::layout_flex_dispatch;

/// Extension trait for [`FormattingContext::Flex`] operations.
pub trait FlexExt {
    /// `true` when this formatting context is [`Flex`](FormattingContext::Flex).
    fn is_flex(&self) -> bool;

    /// Construct a [`Flex`](FormattingContext::Flex) formatting context.
    fn flex() -> Self;
}

impl FlexExt for FormattingContext {
    fn is_flex(&self) -> bool {
        matches!(self, FormattingContext::Flex)
    }

    fn flex() -> Self {
        FormattingContext::Flex
    }
}

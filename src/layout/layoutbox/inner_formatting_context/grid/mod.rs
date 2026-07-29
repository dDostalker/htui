//! Grid formatting context (reserved for future support).
//!
//! No layout function in this module is registered with
//! [`crate::layout::DEFAULT_LAYOUT`] yet. The module exists so that
//! [`FormattingContext::Grid`](super::FormattingContext::Grid) has a natural
//! home once a real CSS grid implementation lands.

use super::FormattingContext;

/// Extension trait for [`FormattingContext::Grid`] operations.
pub trait GridExt {
    /// `true` when this formatting context is [`Grid`](FormattingContext::Grid).
    fn is_grid(&self) -> bool;

    /// Construct a [`Grid`](FormattingContext::Grid) formatting context.
    fn grid() -> Self;
}

impl GridExt for FormattingContext {
    fn is_grid(&self) -> bool {
        matches!(self, FormattingContext::Grid)
    }

    fn grid() -> Self {
        FormattingContext::Grid
    }
}

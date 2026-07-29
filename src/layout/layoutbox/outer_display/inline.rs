//! Helpers for [`OuterDisplay::Inline`](super::OuterDisplay::Inline).

use super::OuterDisplay;

/// Extension trait for [`OuterDisplay::Inline`](super::OuterDisplay::Inline).
#[allow(dead_code)]
pub trait InlineExt {
    /// `true` when this box's outer display is [`Inline`](super::OuterDisplay::Inline)
    /// (participates in its parent's inline flow).
    fn is_inline(&self) -> bool;

    /// Construct an [`Inline`](super::OuterDisplay::Inline) outer display.
    fn inline_display() -> Self;
}

impl InlineExt for OuterDisplay {
    fn is_inline(&self) -> bool {
        matches!(self, OuterDisplay::Inline)
    }

    fn inline_display() -> Self {
        OuterDisplay::Inline
    }
}
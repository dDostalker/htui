//! Helpers for [`OuterDisplay::Block`](super::OuterDisplay::Block).

use super::OuterDisplay;

/// Extension trait for [`OuterDisplay::Block`](super::OuterDisplay::Block).
#[allow(dead_code)]
pub trait BlockExt {
    /// `true` when this box's outer display is [`Block`](super::OuterDisplay::Block)
    /// (the default — generates its own line in the parent's flow).
    fn is_block(&self) -> bool;

    /// Construct a [`Block`](super::OuterDisplay::Block) outer display.
    fn block() -> Self;
}

impl BlockExt for OuterDisplay {
    fn is_block(&self) -> bool {
        matches!(self, OuterDisplay::Block)
    }

    fn block() -> Self {
        OuterDisplay::Block
    }
}
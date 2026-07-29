//! Helpers for [`BoxKind::Normal`](super::BoxKind::Normal).

use super::BoxKind;

/// Extension trait for [`BoxKind::Normal`](super::BoxKind::Normal) operations.
#[allow(dead_code)]
pub trait NormalExt {
    /// `true` when this box is a [`Normal`](super::BoxKind::Normal) block
    /// container.
    fn is_normal(&self) -> bool;

    /// Construct a [`Normal`](super::BoxKind::Normal) box.
    fn normal() -> Self;
}

impl NormalExt for BoxKind {
    fn is_normal(&self) -> bool {
        matches!(self, BoxKind::Normal)
    }

    fn normal() -> Self {
        BoxKind::Normal
    }
}
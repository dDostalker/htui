//! Helpers for [`BoxKind::Anonymous`](super::BoxKind::Anonymous).

use super::BoxKind;

/// Extension trait for [`BoxKind::Anonymous`](super::BoxKind::Anonymous) operations.
#[allow(dead_code)]
pub trait AnonymousExt {
    /// `true` when this box is an [`Anonymous`](super::BoxKind::Anonymous)
    /// wrapper (produced by the layout engine itself).
    fn is_anonymous(&self) -> bool;

    /// Construct an [`Anonymous`](super::BoxKind::Anonymous) wrapper box.
    fn anonymous() -> Self;
}

impl AnonymousExt for BoxKind {
    fn is_anonymous(&self) -> bool {
        matches!(self, BoxKind::Anonymous)
    }

    fn anonymous() -> Self {
        BoxKind::Anonymous
    }
}
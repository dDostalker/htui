//! Helpers for [`BoxKind::Replaced`](super::BoxKind::Replaced).

use super::{BoxKind, ReplacedContent};

/// Extension trait for [`BoxKind::Replaced`](super::BoxKind::Replaced) operations.
#[allow(dead_code)]
pub trait ReplacedExt {
    /// `true` when this box is a [`Replaced`](super::BoxKind::Replaced) element
    /// (i.e. content is supplied outside the DOM, like an `<img>` raster).
    fn is_replaced(&self) -> bool;

    /// Wrap a [`ReplacedContent`] payload into a [`Replaced`](super::BoxKind::Replaced)
    /// variant.
    fn replaced(content: ReplacedContent) -> Self;

    /// Returns the inner [`ReplacedContent`] if this box is replaced,
    /// `None` otherwise.
    fn as_replaced(&self) -> Option<&ReplacedContent>;
}

impl ReplacedExt for BoxKind {
    fn is_replaced(&self) -> bool {
        matches!(self, BoxKind::Replaced(_))
    }

    fn replaced(content: ReplacedContent) -> Self {
        BoxKind::Replaced(content)
    }

    fn as_replaced(&self) -> Option<&ReplacedContent> {
        match self {
            BoxKind::Replaced(content) => Some(content),
            _ => None,
        }
    }
}
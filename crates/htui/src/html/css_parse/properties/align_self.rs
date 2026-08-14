//! `align-self` resolution.

use super::super::values::AlignItems;

/// `align-self` resolution result.
///
/// The CSS property is either the keyword `auto` (inherit the parent's
/// `align-items`) or any of the [`AlignItems`] values. Modeled as a sum
/// type so the layout doesn't need to re-parse the raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignSelf {
    /// Inherit the container's `align-items`.
    Auto,
    /// Explicit cross-axis alignment for this item.
    Value(AlignItems),
}

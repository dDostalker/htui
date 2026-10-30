//! Inner formatting context of a box — how a box arranges its own children.
//!
//! Independent of the outer display role (see
//! [`OuterDisplay`](crate::layout::OuterDisplay)): a flex container is
//! [`FormattingContext::Flex`] regardless of whether its outer role is block
//! (`display: flex`) or inline (`display: inline-flex`). Browser engines treat
//! the two axes as orthogonal; this enum encodes the same separation.
//!
//! ## Variants and their implementations
//!
//! - [`FormattingContext::Flow`] — Default block / inline flow. Implementation
//!   lives in [`flow`].
//! - [`FormattingContext::Flex`] — CSS flexbox container. Implementation
//!   lives in [`flex`].
//! - [`FormattingContext::Table`] — Internal table formatting context. Geometry
//!   is assigned by the table algorithm.
//! - [`FormattingContext::Grid`] — CSS grid container. Lives in
//!   [`grid`].

pub mod flex;
pub mod flow;
pub mod grid;

/// Formatting context established by a box for its children.
///
/// `Grid` is implemented by the CSS Grid formatting context in [`grid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormattingContext {
    /// Block / inline formatting context — children are stacked by the
    /// default block flow (or, for inline content, laid out by the inline
    /// layout pass once it exists).
    #[default]
    Flow,
    /// Flex formatting context — direct children are flex items positioned
    /// by `flex-direction` / `flex-wrap` / `justify-content` / `align-items`.
    Flex,
    /// Table formatting context used by generated `<table>` boxes.
    Table,
    /// Grid formatting context. display: grid / inline-grid classify
    /// here and the geometry phase dispatches them to [`grid`].
    Grid,
}

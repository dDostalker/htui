//! Inner formatting context of a box — how a box arranges its own children.
//!
//! Independent of the outer display role (see
//! [`super::outer_display`]): a flex container is
//! [`FormattingContext::Flex`] regardless of whether its outer role is block
//! (`display: flex`) or inline (`display: inline-flex`). Browser engines treat
//! the two axes as orthogonal; this enum encodes the same separation.
//!
//! ## Variants and their implementations
//!
//! - [`FormattingContext::Flow`] — Default block / inline flow. Implementation
//!   lives in [`flow`](self::flow).
//! - [`FormattingContext::Flex`] — CSS flexbox container. Implementation
//!   lives in [`flex`](self::flex).
//! - [`FormattingContext::Grid`] — CSS grid container (reserved). Lives in
//!   [`grid`](self::grid).
//!
//! ## Module layout
//!
//! ```text
//! inner_formatting_context/
//! ├── mod.rs     ← enum + FlowExt / FlexExt / GridExt re-exports (this file)
//! ├── flow/      ← Flow implementation (block layout + element handlers)
//! │   └── mod.rs (also defines FlowExt + impl FormattingContext::Flow)
//! ├── flex/      ← Flex implementation (CSS flexbox algorithm)
//! │   └── mod.rs (also defines FlexExt + impl FormattingContext::Flex)
//! └── grid/      ← Grid implementation (reserved)
//!     └── mod.rs (also defines GridExt + impl FormattingContext::Grid)
//! ```

pub mod flow;
pub mod flex;
pub mod grid;

pub use flow::FlowExt;
pub use flex::FlexExt;
pub use grid::GridExt;

/// Formatting context established by a box for its children.
///
/// `Grid` is reserved for a future CSS grid implementation.
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
    /// Grid formatting context. Reserved for future support; the layout
    /// pipeline does not yet produce boxes with this context.
    Grid,
}

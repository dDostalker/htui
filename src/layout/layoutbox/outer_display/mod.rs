//! Outer `display` role of a box.
//!
//! Mirrors the two-axis CSS display model: a box has one outer display role
//! (how it participates in its parent's formatting context) and one inner
//! formatting context (how it lays out its own children). These are
//! independent: a flex container can be inline (`display: inline-flex`) and
//! still arrange its children by flexbox; a block-level `<div>` with no
//! children has the same outer display as an `<img>`.
//!
//! ## Variants
//!
//! - [`OuterDisplay::Block`] — The box generates its own line in its
//!   parent's flow. This is the implicit default when `display` is missing.
//! - [`OuterDisplay::Inline`] — The box participates in its parent's
//!   inline formatting context (`inline` / `inline-block` / `inline-flex`).
//!
//! The renderer is currently block-only, so `Inline` is stored but treated as
//! `Block` at the layout layer.
//!
//! ## Module layout
//!
//! ```text
//! outer_display/
//! ├── mod.rs       ← enum (this file)
//! ├── block.rs     ← impl Block variant helpers
//! └── inline.rs    ← impl Inline variant helpers
//! ```

mod block;
mod inline;

#[allow(unused_imports)]
pub use block::BlockExt;
#[allow(unused_imports)]
pub use inline::InlineExt;

/// Outer display role of a layout box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OuterDisplay {
    /// `display: block` (and the implicit default when `display` is missing).
    /// The box generates its own line in its parent's flow.
    #[default]
    Block,
    /// `display: inline` / `inline-block` / `inline-flex` / `inline-grid`. The
    /// box participates in its parent's inline formatting context.
    Inline,
}
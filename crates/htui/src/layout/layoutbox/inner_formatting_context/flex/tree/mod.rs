//! Flex layout algorithm — entry point, container layout, item placement.
//!
//! Implements CSS flexbox for block-level elements. The entry point is
//! [`dispatch::layout_flex_dispatch`], registered *first* in
//! [`crate::layout::DEFAULT_LAYOUT`] so it preempts the element-specific
//! block layout whenever the node has `display: flex`. For non-flex nodes
//! it returns `None`, letting the registry fall through to the next claimer
//! — this is what lets flex work uniformly on `<div>`, `<section>`,
//! `<header>`, etc. without modifying any of their layout functions.
//!
//! ## Scope (v1)
//!
//! Supported CSS properties (full grammar; values are case-insensitive):
//!
//! - `display: flex`
//! - `flex-direction: row | row-reverse | column | column-reverse`
//! - `flex-wrap: nowrap | wrap | wrap-reverse`
//! - `justify-content: flex-start | flex-end | center | space-between |
//!   space-around | space-evenly`
//! - `align-items: stretch | flex-start | flex-end | center | baseline`
//!   (baseline aligns each item's first text row on the line's shared
//!   baseline — the terminal cell-grid analogue of the CSS baseline)
//! - `align-self: auto | <same as align-items>`
//! - `align-content: <same as justify-content>`
//! - `flex-grow`, `flex-shrink` (non-negative numbers)
//! - `flex-basis: auto | <length>`
//! - `order` (integer)
//! - `gap`, `row-gap`, `column-gap`
//!
//! Contiguous raw text children of a flex container are wrapped into
//! anonymous flex items (CSS Flexbox §4).
//!
//! Not supported:
//!
//! - `display: inline-flex` participates as a block-level box (the value is
//!   accepted and blockified; the renderer is block-only)
//! - `align-items: stretch` when the container has no explicit `height`
//!   (cross-axis size would be ambiguous — see the stretch section below)
//!
//! ## Algorithm
//!
//! Each flex container is laid out in two phases:
//!
//! 1. **Measure** every block child's `flex-basis` (resolved against the
//!    main-axis size), falling back to its CSS `width` / `height`, then to
//!    the text content width when nothing else is set.
//! 2. **Distribute** remaining main-axis space across a line (or wrap onto
//!    additional lines) according to `flex-grow` / `flex-shrink`, then
//!    apply `justify-content` to the line.
//!
//! The cross-axis size of each line is the max natural cross-axis size of
//! the items on it. `align-items: stretch` enlarges items to the line's
//! cross-axis size **only when the container has an explicit `height`** —
//! this matches the v1 contract: without a known cross-axis size, stretch
//! would otherwise silently change the container's height.
//!
//! ## Children policy
//!
//! Only the container's **block** children are flex items. The container's
//! own text / inline children (`Text`, `Span`, `A`, `Em`) are still
//! collected as the container's inline runs and rendered as a single row
//! of the container itself; they are **not** treated as anonymous flex
//! items. This is the v1 limitation — see the [scope](#scope-v1) section
//! above.
//!
//! ## Module layout
//!
//! ```text
//! tree/
//! ├── mod.rs        ← re-exports + FlexItemPrep (this file)
//! ├── dispatch.rs   ← layout_flex_dispatch + node capability predicates
//! ├── container.rs  ← layout_flex_container (the orchestrator)
//! ├── items.rs      ← layout_flex_items (per-line placement)
//! ├── prepare.rs    ← FlexItemPrep construction + measurement helpers
//! └── algorithm.rs  ← wrap / distribute / justify / align algorithms
//! ```

mod algorithm;
mod container;
mod dispatch;
mod items;
mod prepare;

pub use dispatch::layout_flex_dispatch;
pub(crate) use prepare::natural_grid_item_size;

//! Shared block-layout primitives.
//!
//! All block-level layout functions in the [`super`] flow orchestrator share
//! the same outer-geometry computation, the same cursor-driven border-box
//! reservation, and the same `LayoutBlockOptions` shape for element-specific
//! differences. This module owns the helpers so each per-element handler
//! stays focused on its own concerns (prefix runs, indent).
//!
//! ```text
//! layout_block/
//! ├── mod.rs        ← re-exports (this file)
//! ├── geometry.rs   ← OuterGeometry + width/left/reserve/rect helpers
//! ├── options.rs    ← LayoutBlockOptions (per-element differences)
//! └── core.rs       ← layout_block() main pipeline
//! ```

mod core;
mod geometry;
mod options;

pub use core::layout_block;
pub use geometry::{
    OuterGeometry, block_left, build_border_rect, compute_outer_geometry, constrained_block_width,
    reserve_border_box, resolve_block_width,
};
pub use options::LayoutBlockOptions;

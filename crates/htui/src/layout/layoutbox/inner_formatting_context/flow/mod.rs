//! Block / inline-block formatting context implementation.
//!
//! Every layout function in this module implements [`LayoutFn`](crate::layout::LayoutFn)
//! for an element type that participates in the default block flow. Layouts
//! here treat the containing block as a vertical sequence of rows; every
//! block reserves `height` rows starting at the current cursor and advances
//! the cursor past the box's border-box plus `margin.bottom`.
//!
//! Flex, table, and replaced boxes are selected by the geometry dispatcher
//! before normal-flow handlers in this module are consulted.
//!
//! ## Module layout
//!
//! ```text
//! flow/
//! ├── mod.rs           ← re-exports (this file)
//! ├── tree.rs          ← layout_tree() orchestrator
//! ├── layout_block/    ← shared layout_block() + OuterGeometry + LayoutBlockOptions
//! │   ├── mod.rs       ←   re-exports
//! │   ├── geometry.rs  ←   outer-geometry helpers
//! │   ├── options.rs   ←   LayoutBlockOptions
//! │   └── core.rs      ←   layout_block() main pipeline
//! ├── aside.rs / body.rs / div.rs / generic.rs / head.rs
//! ├── header_footer.rs / html.rs / list.rs / p.rs
//! ├── img.rs
//! └── table/           ← table-element layout (see `table/mod.rs`)
//!     ├── mod.rs       ←   re-exports + table constants
//!     ├── table.rs     ←   layout_table orchestrator
//!     ├── row_group.rs ←   layout_row_group pass-through
//!     ├── tr.rs        ←   layout_tr pass-through
//!     ├── td.rs        ←   layout_td + layout_th pass-through
//!     ├── cell.rs      ←   CellInfo + layout_cell
//!     └── measure.rs   ←   collect_rows + collect_cells + cell_text_width +
//!                          compute_column_widths
//! ```
//!
//! `collect_runs` lives at `crate::layout::layoutbox::collect_runs` because it
//! is shared with the flex layout (see the flex submodule).

mod a;
mod elements;
mod generic;
mod head;
mod img;
mod layout_block;
mod list;
mod table;
mod tree;

pub use a::layout_a;
pub use elements::layout_standard_block;
pub use generic::layout_generic;
pub use head::{layout_hn, layout_meta, layout_title};
pub use img::{layout_img, layout_img_with_rect};
pub use layout_block::{
    LayoutBlockOptions, OuterGeometry, block_left, build_border_rect, compute_outer_geometry,
    constrained_block_width, layout_block, reserve_border_box, resolve_block_width,
};
pub use list::{layout_li, layout_ol, layout_ul};
pub use table::{layout_row_group, layout_table, layout_td, layout_th, layout_tr};
pub use crate::html::css_parse::FontContext;
pub(crate) use tree::dispatch_flow_geometry;
pub use tree::{dispatch, layout_children, layout_tree};
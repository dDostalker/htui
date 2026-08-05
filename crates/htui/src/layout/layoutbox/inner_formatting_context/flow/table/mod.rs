//! Table layout — produces a 2-D grid of cells from a `<table>` subtree.
//!
//! Layout walks the table once to measure column widths (max content width
//! per column across all rows), then lays out each cell at an explicit
//! `(x, y, width, height)` rect so every column in a row shares the same
//! starting `x` and ending `width` — matching the columnar alignment a
//! `<table>` requires.
//!
//! ## Scope
//!
//! - Supported elements: `<table>`, `<thead>`, `<tbody>`, `<tfoot>`, `<tr>`,
//!   `<th>`, `<td>`. `<th>` is rendered like `<td>`; the user-agent default
//!   `font-weight: bold; text-align: center;` is reproduced either by the
//!   user adding CSS (`th { ... }`) or by the cascade step.
//! - `<caption>`, `<col>`, `<colgroup>` are parsed into the DOM but act as
//!   sinks at layout time (no boxes, no rows).
//! - `<thead>` / `<tbody>` / `<tfoot>` are transparent row-grouping wrappers
//!   — their children are flattened into the table's row list.
//! - `colspan` / `rowspan` are ignored: each cell occupies one column and
//!   one row.
//!
//! ## Module layout
//!
//! ```text
//! table/
//! ├── mod.rs       ← re-exports (this file)
//! ├── table.rs     ← layout_table orchestrator
//! ├── row_group.rs ← layout_row_group pass-through
//! ├── tr.rs        ← layout_tr pass-through
//! ├── td.rs        ← layout_td + layout_th pass-through
//! ├── cell.rs      ← CellInfo + layout_cell
//! └── measure.rs   ← collect_rows + collect_cells + cell_text_width +
//!                   compute_column_widths + table constants
//! ```

mod cell;
mod measure;
mod row_group;
mod table;
mod td;
mod tr;

pub use row_group::layout_row_group;
pub use table::layout_table;
pub use td::{layout_td, layout_th};
pub use tr::layout_tr;

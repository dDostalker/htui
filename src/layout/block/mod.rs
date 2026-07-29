//! Block layout.
//!
//! Every function in this module implements `LayoutFn` for an element type
//! that participates in the default block flow. Layouts here treat the
//! containing block as a vertical sequence of rows; every block reserves
//! `height` rows starting at the current cursor and advances the cursor
//! past the box's border-box plus `margin.bottom`.
//!
//! `flex` and `grid` live next to this module and are conceptually
//! orthogonal: a flex container is a block-level element whose *children*
//! are arranged by the flex algorithm, while the block functions here are
//! the layout algorithms for the children themselves.

pub mod aside;
pub mod body;
pub mod div;
pub mod head;
pub mod header_footer;
pub mod html;
pub mod img;
pub mod list;
pub mod p;
pub mod table;

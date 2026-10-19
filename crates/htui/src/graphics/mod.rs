//! Terminal graphics primitives: the character-cell model and the shared
//! canvas.
//!
//! This module sits **below** the layout and terminal layers. The layout /
//! paint pipeline writes [`Block`] cells into a [`TermLayout`] canvas; the
//! `tui` layer then renders that canvas to the terminal. Keeping the cell /
//! width / image / canvas types here (instead of under `tui`) makes the
//! dependency direction explicit — `layout → graphics → (tdom, config,
//! resources)` — and leaves `tui` as the thin terminal-IO layer on top.

mod canvas;
mod cell;
mod image;
mod style;
mod text;

pub use canvas::{InputRenderState, TermLayout};
pub(crate) use cell::styled_string;
pub use cell::{Block, FontStyle};
pub(crate) use image::image_intrinsic_size;
pub use image::{
    ClipRect, IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, ImageCache, ImageError, Img,
    render_image_at, render_image_at_slice,
};
pub use style::block_style;
pub use text::{char_width, grapheme_width};

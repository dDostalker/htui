mod canvas;
mod cell;
mod image;
mod input;
mod render;
mod style;
mod text;

pub(crate) use image::image_intrinsic_size;
pub use canvas::{InputRenderState, TermLayout};
pub use cell::{Block, FontStyle};
pub use image::{
    IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, ClipRect, ImageCache, Img,
    render_image_at, render_image_at_slice,
};
pub use input::{
    DirtyInfo, InputState, RenderCmd, apply_input_key, handle_click, selection_range,
    trigger_refresh,
};
pub use render::is_rendered_as_bare_space;
pub use style::block_style;
pub use text::{char_width, grapheme_width};

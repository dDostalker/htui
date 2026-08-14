mod canvas;
mod cell;
mod image;
mod input;
mod render;
mod style;
mod text;

pub use canvas::{InputRenderState, TermLayout};
pub use cell::{Block, FontStyle};
pub(crate) use image::image_intrinsic_size;
pub use image::{
    ClipRect, IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, ImageCache, ImageError, Img,
    render_image_at, render_image_at_slice,
};
pub(crate) use input::reapply_console_modes;
pub use input::{
    DirtyInfo, InputState, KEY_GATE_TIMEOUT, KeyGate, OverlayKind, RenderCmd, SelectOpenState,
    SelectionMode, apply_focus_requests, apply_input_key, apply_input_word_key, apply_textarea_key,
    collect_input_ids, handle_click, handle_mouse_down, handle_mouse_move, is_input_field_key,
    key_event_info, next_word_boundary, number_input_allows_char, paste_into_field,
    prev_word_boundary, selection_range, trigger_refresh,
};
pub use render::is_rendered_as_bare_space;
pub use style::block_style;
pub use text::{char_width, grapheme_width};

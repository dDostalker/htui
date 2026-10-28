//! Terminal-side API surface.
//!
//! The character-cell model, width helpers and the [`TermLayout`] canvas live
//! in [`crate::graphics`] (below this layer); this module re-exports them so
//! host code keeps the familiar `htui::tui::graph_api::…` paths, and adds the
//! terminal-IO pieces: the input-loop state machine ([`InputState`]) and the
//! canvas renderers (ANSI emission, text-selection overlay).

mod input;
mod render;

pub use crate::graphics::{
    Block, ClipRect, FontStyle, IMAGE_PIXELS_PER_CELL_COL, IMAGE_PIXELS_PER_CELL_ROW, ImageCache,
    ImageError, Img, InputRenderState, TermLayout, block_style, char_width, grapheme_width,
    render_image_at, render_image_at_slice,
};
pub(crate) use input::reapply_console_modes;
pub use input::{
    ANIM_POLL_MS, DirtyInfo, DrainedBatch, InputState, KEY_GATE_TIMEOUT, KeyGate, OverlayKind,
    RenderCmd, ScrollAnim, ScrollIntoViewBlock, ScrollTarget, SelectOpenState, SelectionMode,
    active_scroll_target, apply_focus_requests, apply_input_key, apply_input_word_key,
    apply_textarea_key, collect_input_ids, ease_out_cubic, handle_click, handle_mouse_down,
    handle_mouse_move, handle_mouse_up, is_input_field_key, key_event_info, next_word_boundary,
    number_input_allows_char, paste_into_field, prev_word_boundary, scroll_anim_duration,
    scroll_into_view_target, selection_range, step_target, trigger_refresh,
};
pub use render::is_rendered_as_bare_space;

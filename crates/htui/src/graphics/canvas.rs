use std::collections::HashMap;

use compact_str::CompactString;

use super::{cell::Block, image::ImageCache};
use crate::tdom::node::NodeId;

/// Render-time state of one `<input>` field.
///
/// The input loop updates this through `RenderCmd::RenderInput`
/// (crate::tui::graph_api::RenderCmd); the paint phase reads it to draw the
/// field's value and caret. An absent entry falls back to the DOM `value`
/// attribute and an unfocused look, so an untouched document renders without
/// any engine-side state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputRenderState {
    /// Current field value (the DOM `value` attribute is the initial value).
    pub value: CompactString,
    /// Caret position in `value` (byte index into the displayed string).
    pub cursor: usize,
    /// Whether the field currently holds keyboard focus.
    pub focused: bool,
    /// Selected character range `(start, end)` (character indices, `start <=
    /// end`), painted with a highlight. `None` when nothing is selected.
    pub selection: Option<(usize, usize)>,
}

#[derive(Debug)]
pub struct TermLayout {
    pub layout: Vec<Block>,
    pub columns: u16,
    pub rows: u16,
    pub scroll_y: u32,
    pub rendered_rows: u32,
    pub zoom: f32,
    /// Snapshot of the last viewport actually sent to the terminal, used by
    /// the incremental renderer to emit only changed cells. Empty until the
    /// first [`TermLayout::render_diff_to_bytes`](crate::tui::graph_api::TermLayout::render_diff_to_bytes)
    /// call.
    pub last_frame: Vec<Block>,
    /// Rasterized-image cache shared across redraws so `<img>` sources are
    /// decoded from disk once per (path, size) instead of once per frame.
    pub(crate) image_cache: ImageCache,
    /// `<input>` edit state keyed by node id, written by
    /// [`RenderCmd::RenderInput`](crate::tui::graph_api::RenderCmd) and read by
    /// the input painter. Transient: focused entries hold the live edit
    /// buffer, and an unfocused entry bridges the single blur repaint — every
    /// fresh layout (see `TermLayout::layout_only`) drops unfocused entries,
    /// after which the painter reads the DOM `value` attribute.
    pub input_states: HashMap<NodeId, InputRenderState>,
    /// The `<select>` node whose dropdown is open (rendered as an overlay),
    /// set by the render thread while a dropdown is expanded.
    pub open_select: Option<NodeId>,
}

impl TermLayout {
    /// Terminal columns after applying `zoom` (rounded).
    pub fn effective_columns(&self) -> u16 {
        (self.columns as f32 / self.zoom).round() as u16
    }

    /// Terminal rows after applying `zoom` (rounded).
    pub fn effective_rows(&self) -> u16 {
        (self.rows as f32 / self.zoom).round() as u16
    }

    /// Maximum valid scroll offset for the painted canvas and viewport.
    pub fn max_scroll(&self) -> u32 {
        // The canvas grows to exactly the content height (see
        // `ensure_capacity`), but `rendered_rows` is a monotonic high-water
        // mark and can briefly exceed the buffer (e.g. a re-layout after a
        // terminal grow). A scroll bound past the actual buffer would make
        // the diff renderer slice `layout[scroll_start..]` past its end and
        // underflow — killing the render thread — so the bound must respect
        // the painted canvas height too.
        let canvas_rows = self.layout.len() / (self.columns as usize).max(1);
        let content = self.rendered_rows.saturating_sub(self.rows as u32) as usize;
        canvas_rows.saturating_sub(self.rows as usize).min(content) as u32
    }

    /// Clamp `scroll_y` to the valid range `[0, max_scroll()]`.
    pub fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        if self.scroll_y > max {
            self.scroll_y = max;
        }
    }
}

use super::{cell::Block, image::ImageCache};

/// Upper bound (in screens) the canvas may grow to for scrollable content.
///
/// The canvas starts at one viewport and grows on demand (see
/// [`TermLayout::ensure_capacity`](crate::layout::layout::TermLayout)) so a
/// short document keeps a small buffer; this limit only caps how far tall
/// documents can extend so an unbounded page cannot exhaust memory.
pub(crate) const MAX_SCROLL_MULTIPLIER: usize = 20;

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
}

impl TermLayout {
    pub fn max_cells(&self) -> usize {
        self.columns as usize * self.rows as usize * MAX_SCROLL_MULTIPLIER
    }

    pub fn effective_columns(&self) -> u16 {
        (self.columns as f32 / self.zoom).round() as u16
    }

    pub fn effective_rows(&self) -> u16 {
        (self.rows as f32 / self.zoom).round() as u16
    }

    pub fn max_scroll(&self) -> u32 {
        self.rendered_rows.saturating_sub(self.rows as u32)
    }

    pub fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        if self.scroll_y > max {
            self.scroll_y = max;
        }
    }
}

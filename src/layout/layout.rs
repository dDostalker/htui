use anyhow::{Ok, Result};
use crossterm::terminal;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        inner_formatting_context::flow::layout_tree,
        paint::paint_layout_tree,
    },
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::{Block, FontStyle, ImageCache, TermLayout},
};

#[derive(Debug)]
pub struct TermMap {
    pub columns: u16,
    pub rows: u16,
}

impl TermMap {
    pub fn get_ter_map() -> Result<TermMap> {
        let (columns, rows) = terminal::size()?;
        log::info!("终端尺寸: {}x{}", columns, rows);
        let columns = if columns == 0 { 80 } else { columns };
        let rows = if rows == 0 { 24 } else { rows };
        Ok(TermMap { columns, rows })
    }
}

impl TermLayout {
    pub fn new(map: &TermMap, zoom: f32) -> TermLayout {
        // Buffer up to MAX_SCROLL_MULTIPLIER screens of content so documents
        // taller than the visible viewport (e.g. an image-heavy page that
        // produces 200+ rows) keep their trailing boxes in the canvas and
        // remain reachable through scrolling instead of being silently dropped
        // at the layout boundary.
        let max_cells = map.columns as usize
            * map.rows as usize
            * crate::tui::graph_api::MAX_SCROLL_MULTIPLIER;
        let layout = vec![Block::default(); max_cells];
        TermLayout {
            layout,
            last_frame: Vec::new(),
            columns: map.columns,
            rows: map.rows,
            scroll_y: 0,
            rendered_rows: 0,
            zoom,
            image_cache: ImageCache::new(),
        }
    }
}

impl TermLayout {
    /// Render the DOM tree into the canvas (virtual, may exceed terminal size)
    /// and return the laid-out box forest.
    ///
    /// The canvas is reset at the start of every call so a single
    /// [`TermLayout`] allocated by [`new`](Self::new) can be re-used across
    /// redraws — callers do not need to drop and reallocate the layout Vec
    /// each frame.
    ///
    /// Two-phase pipeline:
    /// 1. Reset every cell to a blank space so stale cells from a previous
    ///    render at rows the new tree does not touch don't bleed through.
    /// 2. [`layout_tree`] walks the DOM through `DEFAULT_LAYOUT` and builds a
    ///    `Vec<LayoutBox>` rooted at `root_id`. Each block claims a row range
    ///    inside the containing block.
    /// 3. [`paint_layout_tree`] writes the resulting rects into `self.layout`
    ///    using each box's own coordinates — no shared cursor.
    ///
    /// The returned `Vec<LayoutBox>` is the caller's to keep (e.g. shared
    /// with the hit-test thread of the event system).
    pub fn draw(
        &mut self,
        arena: &NodeArena,
        root_id: NodeId,
        term_map: &TermMap,
    ) -> Result<Vec<LayoutBox>> {
        self.reset_canvas();

        let root = &arena[root_id];
        assert_eq!(root.node_type, NodeType::Global);
        let containing = BoxConstraint::root(term_map);
        let mut cursor = Cursor::new();
        let boxes = layout_tree(
            arena,
            root_id,
            containing,
            &mut cursor,
            term_map.columns as usize,
            term_map.rows as usize,
        );
        paint_layout_tree(&boxes, arena, self);
        Ok(boxes)
    }

    /// Re-paint the canvas from an existing layout without re-laying out.
    ///
    /// Resets every cell and paints `boxes` against `arena`. Used by the
    /// render thread on `RenderCmd::Hover`: the arena is a fresh cascade for
    /// the CSS `:hover` pseudo-class, and since paint reads style from the
    /// arena this picks up `:hover` backgrounds without re-layout.
    pub fn repaint_from(&mut self, arena: &NodeArena, boxes: &[LayoutBox]) {
        self.reset_canvas();
        paint_layout_tree(boxes, arena, self);
    }

    /// Reset every cell to a blank space and clear `rendered_rows`.
    ///
    /// Re-uses each `Block`'s existing `word` buffer (via `clear` +
    /// `push(' ')`) so the amortised cost is just field writes — no fresh
    /// `String` allocations per cell. We intentionally do *not* use
    /// `Block::default()` + clone here: `default()` itself allocates a
    /// 1-byte `String` for the `" "` word, so cloning it once per cell
    /// would re-introduce the per-frame heap traffic this loop was
    /// designed to avoid.
    fn reset_canvas(&mut self) {
        for cell in self.layout.iter_mut() {
            cell.word.clear();
            cell.word.push(' ');
            cell.fcolor = None;
            cell.bcolor = None;
            cell.style = FontStyle::empty();
        }
        self.rendered_rows = 0;
    }
}

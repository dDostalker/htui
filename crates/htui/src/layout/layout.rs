use anyhow::{Ok, Result};
use crossterm::terminal;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox, Rect,
        inner_formatting_context::flow::layout_tree,
        paint::{paint_layout_tree, paint_layout_tree_region},
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
        // The canvas starts at a single viewport and grows on demand (see
        // [`ensure_capacity`](Self::ensure_capacity)) up to
        // `MAX_SCROLL_MULTIPLIER` screens. A short document therefore keeps a
        // small buffer instead of pre-allocating the whole scroll capacity up
        // front (which is `columns × rows × 20` cells on a tall page).
        let viewport_cells = map.columns as usize * map.rows as usize;
        let layout = vec![Block::default(); viewport_cells];
        TermLayout {
            layout,
            last_frame: Vec::new(),
            columns: map.columns,
            rows: map.rows,
            scroll_y: 0,
            rendered_rows: 0,
            zoom,
            image_cache: ImageCache::new(),
            input_states: std::collections::HashMap::new(),
            open_select: None,
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
        let boxes = self.layout_only(arena, root_id, term_map)?;
        self.ensure_capacity(&boxes);
        paint_layout_tree(&boxes, arena, self);
        Ok(boxes)
    }

    /// Run only the layout pass (no canvas reset / paint) and return the
    /// laid-out box forest.
    ///
    /// The shared first half of [`draw`](Self::draw): incremental renders run
    /// it and then decide how much of the canvas to repaint, so a small DOM
    /// mutation does not have to clear and re-paint the whole buffer. The pass
    /// also drops stale unfocused [`input_states`](Self::input_states) entries
    /// — a fresh layout re-establishes the arena as the display source of
    /// truth for unfocused `<input>` fields.
    pub fn layout_only(
        &mut self,
        arena: &NodeArena,
        root_id: NodeId,
        term_map: &TermMap,
    ) -> Result<Vec<LayoutBox>> {
        // Unfocused `input_states` entries only bridge the single repaint
        // right after a blur (so the field doesn't flash its pre-commit DOM
        // value); once a fresh layout runs, the arena's `value` attribute is
        // the display truth and stale cache entries must not survive. Focused
        // entries keep the live edit buffer.
        self.input_states.retain(|_, st| st.focused);
        let root = &arena[root_id];
        assert_eq!(root.node_type, NodeType::Global);
        let containing = BoxConstraint::root(term_map);
        let mut cursor = Cursor::new();
        Ok(layout_tree(
            arena,
            root_id,
            containing,
            &mut cursor,
            term_map.columns as usize,
            term_map.rows as usize,
        ))
    }

    /// Repaint only the cells inside `region` from an existing layout,
    /// leaving the rest of the canvas untouched.
    ///
    /// Backs the incremental render path: the caller has re-run the layout
    /// and verified every box rect is unchanged against the cached layout, so
    /// only the region's cells can differ from the previous frame. The region
    /// is cleared first (stale text from a shortened value would otherwise
    /// stay on screen), then re-painted with writes gated to it.
    pub fn repaint_region(&mut self, arena: &NodeArena, boxes: &[LayoutBox], region: Rect) {
        self.ensure_capacity(boxes);
        self.clear_region(region);
        paint_layout_tree_region(boxes, arena, self, region);
    }

    /// Re-paint the canvas from an existing layout without re-laying out.
    ///
    /// Resets every cell and paints `boxes` against `arena`. Used by the
    /// render thread on `RenderCmd::Hover`: the arena is a fresh cascade for
    /// the CSS `:hover` pseudo-class, and since paint reads style from the
    /// arena this picks up `:hover` backgrounds without re-layout.
    pub fn repaint_from(&mut self, arena: &NodeArena, boxes: &[LayoutBox]) {
        self.reset_canvas();
        self.ensure_capacity(boxes);
        paint_layout_tree(boxes, arena, self);
    }

    /// Grow the canvas on demand so it can hold every box in `boxes`.
    ///
    /// The buffer starts at one viewport ([`new`](Self::new)) and only grows
    /// here, to exactly the height the layout needs — a short document never
    /// pays for the full scroll capacity. Growth is clamped to the
    /// `MAX_SCROLL_MULTIPLIER` limit so an unboundedly tall document cannot
    /// exhaust memory, and the buffer never shrinks (content height is
    /// monotonic within a session). Freshly added cells are blank, so the
    /// subsequent paint pass fills them exactly like the rest of the buffer.
    fn ensure_capacity(&mut self, boxes: &[LayoutBox]) {
        fn walk(boxes: &[LayoutBox], bottom: &mut usize) {
            for b in boxes {
                *bottom = (*bottom).max(b.rect.y.saturating_add(b.rect.height.max(1)));
                walk(&b.children, bottom);
            }
        }
        let mut bottom = self.rows as usize;
        walk(boxes, &mut bottom);

        let cols = self.columns as usize;
        let needed = bottom
            .saturating_mul(cols)
            .min(self.max_cells())
            .max(cols.saturating_mul(self.rows as usize));
        if needed > self.layout.len() {
            self.layout.resize(needed, Block::default());
        }
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
            blank_cell(cell);
        }
        self.rendered_rows = 0;
    }

    /// Blank every cell inside `region` (the incremental-repaint analogue of
    /// [`reset_canvas`](Self::reset_canvas), scoped to one rectangle).
    fn clear_region(&mut self, region: Rect) {
        let cols = self.columns as usize;
        let max = self.layout.len();
        let x_end = region.x.saturating_add(region.width);
        let y_end = region.y.saturating_add(region.height);
        for y in region.y..y_end {
            let row_start = y.saturating_mul(cols);
            if row_start >= max {
                break;
            }
            for x in region.x..x_end {
                let idx = row_start.saturating_add(x);
                if idx >= max {
                    break;
                }
                blank_cell(&mut self.layout[idx]);
            }
        }
    }
}

/// Clear one canvas cell in place (reuses the word buffer).
fn blank_cell(cell: &mut Block) {
    cell.word.clear();
    cell.word.push(' ');
    cell.fcolor = None;
    cell.bcolor = None;
    cell.style = FontStyle::empty();
}

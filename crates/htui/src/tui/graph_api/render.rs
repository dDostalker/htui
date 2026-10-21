use std::{fmt::Display, io::Write};

use crossterm::{
    cursor::MoveTo,
    style::Color,
    terminal::{Clear, ClearType},
};

use crate::graphics::{Block, FontStyle, TermLayout, styled_string};

impl Display for TermLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cols = self.columns as usize;
        let rows = self.rows as usize;
        let sy = self.scroll_y as usize;
        for r in 0..rows {
            let start = sy
                .saturating_add(r)
                .saturating_mul(cols)
                .min(self.layout.len());
            let end = start.saturating_add(cols).min(self.layout.len());
            for cell in self.layout[start..end].iter() {
                if cell.word != " " {
                    write!(f, "{}", cell)?;
                }
            }
        }
        write!(f, "")
    }
}
impl TermLayout {
    /// Overlay the page-selection highlight on every cell inside the line
    /// bands `lines` — `(row, col_start, col_end_exclusive)` triples, the
    /// same shape the render thread's `TextSelection` handler computes.
    ///
    /// The paint phase knows nothing about the selection, so this is applied
    /// after each repaint. Filler cells (empty word, the second half of a wide
    /// glyph) get the background too so the highlight reads as a continuous
    /// band; the diff renderer skips them, so only glyph-bearing cells change
    /// the output.
    ///
    /// An existing RGB cell background is blended with `selection_bg` rather
    /// than replaced: background-driven widgets (e.g. the filled / empty
    /// halves of a flex progress bar) would otherwise collapse into one
    /// uniform highlight band and become invisible under the selection.
    /// Non-RGB backgrounds (or none) fall back to the plain `selection_bg`.
    pub fn apply_text_selection(&mut self, lines: &[(u32, u32, u32)], selection_bg: Color) {
        let cols = self.columns as usize;
        let max = self.layout.len();
        for &(y, xs, xe) in lines {
            let row_start = (y as usize).saturating_mul(cols);
            if row_start >= max {
                continue;
            }
            let xe = (xe as usize).min(cols);
            for x in (xs as usize)..xe {
                let idx = row_start.saturating_add(x);
                if idx >= max {
                    break;
                }
                self.layout[idx].bcolor =
                    Some(selection_blend(self.layout[idx].bcolor, selection_bg));
            }
        }
    }

    /// Write the incremental frame to `out`.
    ///
    /// Only the cells that changed since the last call are written (see
    /// [`Self::render_diff_to_bytes`]); when nothing changed this is a no-op.
    /// The caller is responsible for flushing `out` once per frame — the
    /// render thread holds a persistent `BufWriter<Stdout>` so each frame
    /// results in exactly one write syscall.
    pub fn render(&mut self, out: &mut impl Write) {
        let bytes = self.render_diff_to_bytes();
        if bytes.is_empty() {
            return;
        }
        if let Err(e) = out.write_all(&bytes) {
            log::error!("Failed to write rendered frame: {}", e);
        }
    }

    /// Render the full visible viewport to an in-memory byte buffer.
    ///
    /// Every visible cell is emitted (unlike [`Self::render`], which only
    /// writes the cells that changed since the last frame) — useful for unit
    /// tests that want to inspect the actual ANSI SGR sequence.
    pub fn render_to_bytes(&self) -> Vec<u8> {
        // Pre-size for roughly one ANSI byte-quartet per visible cell: the
        // incremental run batching makes the exact size unknowable up front,
        // but starting from zero would re-grow the buffer several times on
        // every call (the buffer is dropped after the frame).
        let mut out: Vec<u8> = Vec::with_capacity(
            (self.rows as usize)
                .saturating_mul(self.columns as usize)
                .saturating_mul(4),
        );
        let cols = self.columns as usize;
        let (scroll_start, scroll_end) = self.viewport_range();

        let mut batch = String::new();
        let mut i = scroll_start;
        while let Some(run) = self.next_batch(i, scroll_end, cols, &mut batch) {
            self.queue_batch(&mut out, &run, &batch);
            i = run.end;
        }
        out
    }

    /// Render only the cells that changed since the last call.
    ///
    /// Compares the visible viewport (offset by `scroll_y`) against the
    /// snapshot taken by the previous call and emits `MoveTo` + content for
    /// the runs that differ, skipping runs whose cells are all unchanged.
    /// The snapshot is then refreshed so the next call diffs against the new
    /// screen state. Returns an empty buffer when the viewport is unchanged.
    ///
    /// The first call (or a viewport-size change, e.g. after a resize)
    /// starts with an empty snapshot, so it emits a full-screen clear plus
    /// every content cell.
    pub fn render_diff_to_bytes(&mut self) -> Vec<u8> {
        // Pre-sized like `render_to_bytes`; unchanged frames emit nothing, so
        // the capacity is only a growth hint.
        let mut out: Vec<u8> = Vec::with_capacity(
            (self.rows as usize)
                .saturating_mul(self.columns as usize)
                .saturating_mul(4),
        );
        let cols = self.columns as usize;
        let (scroll_start, scroll_end) = self.viewport_range();
        let viewport_cells = scroll_end.saturating_sub(scroll_start);

        if self.last_frame.len() != viewport_cells {
            // First frame / resize: the screen is in an unknown state (fresh
            // alternate screen), so clear it once and treat every cell as
            // changed. `Block::default()` is a plain space, so blank cells
            // compare equal against the cleared screen and stay silent.
            if let Err(e) = crossterm::queue!(out, Clear(ClearType::All), MoveTo(0, 0)) {
                log::error!("Failed to queue initial screen clear: {}", e);
            }
            self.last_frame = vec![Block::default(); viewport_cells];
        }

        let mut batch = String::new();
        let mut i = scroll_start;
        while let Some(run) = self.next_batch(i, scroll_end, cols, &mut batch) {
            // A batch "changed" when any of its non-empty cells differs from
            // the snapshot. Empty filler cells never matter: they are skipped
            // by the batch scan and the diff renderer only writes
            // glyph-bearing cells.
            let changed = (run.start..run.end).any(|i| {
                !self.layout[i].word.is_empty()
                    && !block_state_eq(&self.last_frame[i - scroll_start], &self.layout[i])
            });
            if changed {
                self.queue_batch(&mut out, &run, &batch);
            }
            i = run.end;
        }

        // Refresh the snapshot for the whole viewport. `clear` + `push_str`
        // reuses each cell's allocation when the new word fits, so unchanged
        // words don't reallocate.
        for (snapshot, cell) in self
            .last_frame
            .iter_mut()
            .zip(self.layout[scroll_start..scroll_end].iter())
        {
            if !block_state_eq(snapshot, cell) {
                snapshot.word.clear();
                snapshot.word.push_str(&cell.word);
                snapshot.fcolor = cell.fcolor;
                snapshot.bcolor = cell.bcolor;
                snapshot.style = cell.style;
            }
        }
        out
    }

    /// The visible viewport as `(scroll_start, scroll_end)` indexes into
    /// `layout`. The scroll position is normally clamped to the canvas, but
    /// the bounds are guarded anyway: a stale out-of-range position (sent
    /// before the clamp republished, or one the old `max_scroll` advertised
    /// for documents taller than the buffer cap) must degrade to an empty
    /// frame instead of indexing past the buffer and panicking the render
    /// thread.
    fn viewport_range(&self) -> (usize, usize) {
        let cols = self.columns as usize;
        let scroll_start = (self.scroll_y as usize)
            .saturating_mul(cols)
            .min(self.layout.len());
        let scroll_end = scroll_start
            .saturating_add((self.rows as usize).saturating_mul(cols))
            .min(self.layout.len());
        (scroll_start, scroll_end)
    }

    /// Scan the next maximal batch of same-styled cells starting at/after
    /// `from`, appending its words to `batch` (cleared first; the buffer is
    /// reused across batches so a styled run no longer allocates a fresh
    /// `String` — it grows to the longest run once and is reused
    /// thereafter). Returns `None` past the viewport end.
    ///
    /// This scan is the single source of truth for the run batching shared
    /// by [`Self::render_to_bytes`] and [`Self::render_diff_to_bytes`]:
    ///
    /// - empty words (filler, the second half of a wide glyph) are skipped
    ///   without ending the batch — a batch may therefore continue past a
    ///   row whose remaining cells are all empty;
    /// - a non-empty cell on a new row ends the batch (emission is
    ///   row-positioned);
    /// - a switch between "bare space" and styled rendering ends the batch,
    ///   as does any styled-cell change of fg / bg / style flags.
    fn next_batch(
        &self,
        from: usize,
        scroll_end: usize,
        cols: usize,
        batch: &mut String,
    ) -> Option<BatchRun> {
        let mut i = from;
        while i < scroll_end && self.layout[i].word.is_empty() {
            i += 1;
        }
        if i >= scroll_end {
            return None;
        }
        let start = i;
        let cell = &self.layout[start];
        let fg = cell.fcolor;
        let bg = cell.bcolor;
        let style = cell.style;
        // A cell is rendered as a "bare" space only when it has no
        // background AND no style flags. Anything styled (even a plain
        // space) must still receive the SGR codes — otherwise spaces
        // between underlined/italic letters would drop the underline.
        let bare = is_rendered_as_bare_space(cell);
        batch.clear();
        batch.push_str(&cell.word);

        i += 1;
        while i < scroll_end {
            let c = &self.layout[i];
            if c.word.is_empty() {
                i += 1;
                continue;
            }
            // `i` is always past `start` here, so a non-empty cell at a row
            // boundary ends the batch (the scan only continues past a row
            // end through skipped empty cells).
            if i.is_multiple_of(cols) {
                break;
            }
            let c_bare = is_rendered_as_bare_space(c);
            if c_bare != bare {
                break;
            }
            if !c_bare && (c.fcolor != fg || c.bcolor != bg || c.style != style) {
                break;
            }
            batch.push_str(&c.word);
            i += 1;
        }
        Some(BatchRun {
            start,
            end: i,
            fg,
            bg,
            style,
            bare,
        })
    }

    /// Write one batch to `out` — `MoveTo` its start cell, then the text
    /// (bare bytes for unstyled spaces, SGR-wrapped otherwise). Batches that
    /// fall outside the visible viewport rows are silently dropped.
    fn queue_batch(&self, out: &mut Vec<u8>, run: &BatchRun, batch: &str) {
        let cols = self.columns as usize;
        let batch_row = run.start / cols;
        let x = (run.start % cols) as u16;
        let y = batch_row.saturating_sub(self.scroll_y as usize);
        if batch_row >= self.scroll_y as usize && y < self.rows as usize {
            if let Err(e) = crossterm::queue!(out, MoveTo(x, y as u16)) {
                log::error!("Failed to queue cursor move to ({}, {}): {}", x, y, e);
            }
            if run.bare {
                out.extend_from_slice(batch.as_bytes());
            } else {
                push_styled(out, batch, run.style, run.fg, run.bg);
            }
        }
    }
}

/// One maximal same-styled run of viewport cells produced by
/// [`TermLayout::next_batch`]: the cell range it covers plus the rendering
/// decision shared by every cell in the run.
struct BatchRun {
    /// Index of the run's first cell.
    start: usize,
    /// One past the run's last cell (a row-boundary cell or `scroll_end`).
    end: usize,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    /// The run renders as bare spaces (no SGR codes).
    bare: bool,
}

/// True when two cells render identically (same text, colors and style).
fn block_state_eq(a: &Block, b: &Block) -> bool {
    a.word == b.word && a.fcolor == b.fcolor && a.bcolor == b.bcolor && a.style == b.style
}

/// Write the SGR escape codes for `style` / `fg` / `bg` plus `text` straight
/// into `out`, avoiding the intermediate `String` that the old
/// `format!("{styled}")` path allocated per styled run. `Vec<u8>` implements
/// `io::Write`, so `write!` dispatches through it and the bytes land directly
/// in the output buffer (the writer is infallible for `Vec<u8>`; the error
/// branch exists only to satisfy the `io::Write` contract).
fn push_styled(
    out: &mut Vec<u8>,
    text: &str,
    style: FontStyle,
    fg: Option<Color>,
    bg: Option<Color>,
) {
    if let Err(e) = write!(out, "{}", styled_string(text, style, fg, bg)) {
        log::error!("Failed to write styled run: {}", e);
    }
}

/// Returns true when a cell should be rendered as a "bare" space without
/// applying any SGR escape codes. Equivalent to the inline check in
/// `TermLayout::render_to_bytes`; exposed for tests.
pub fn is_rendered_as_bare_space(cell: &Block) -> bool {
    cell.word == " " && cell.bcolor.is_none() && cell.style.is_empty()
}

/// Blend an existing cell background with the selection colour.
///
/// RGB backgrounds are averaged so the underlying colour stays visible
/// through the highlight (browser-like translucent `::selection`); anything
/// else (no background, ANSI / fixed / named colours) yields `selection_bg`
/// unchanged.
fn selection_blend(existing: Option<Color>, selection_bg: Color) -> Color {
    let (
        Some(Color::Rgb { r, g, b }),
        Color::Rgb {
            r: sr,
            g: sg,
            b: sb,
        },
    ) = (existing, selection_bg)
    else {
        return selection_bg;
    };
    Color::Rgb {
        r: ((r as u16 + sr as u16) / 2) as u8,
        g: ((g as u16 + sg as u16) / 2) as u8,
        b: ((b as u16 + sb as u16) / 2) as u8,
    }
}

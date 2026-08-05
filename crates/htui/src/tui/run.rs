//! Terminal engine driver — the `render` / `render_with_events` entry-point
//! machinery.
//!
//! Owns the render thread (HTML → layout → paint → ANSI diff output), the
//! hit-test thread (geometry snapshots → shared `HitMap`), the incremental
//! region-repaint helpers, and the input loop that translates keys / mouse
//! events into `RenderCmd`s.

use std::io::{BufWriter, Write, stdout};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crossterm::{
    cursor::{Hide, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    style::Color,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

use crate::{
    events::{AutoRefresh, EventBindings, Geometry, HitMap},
    html::css_parse::{self, CssRule, PseudoClass, Selector, css_into_node_with_hover, reset_css_attr},
    layout::{LayoutBox, Rect},
    layout::layout::TermMap,
    script::ScriptEngine,
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::{
        DirtyInfo, InputRenderState, InputState, RenderCmd, SelectionMode, TermLayout,
        trigger_refresh,
    },
};

fn parse_and_apply(html: &str, css_rules: &CssRule) -> (NodeId, NodeArena) {
    let mut html_parser = crate::html::html_parse::Parser::new(html);
    let (root_id, mut arena, _embedded) = html_parser.parse();
    css_parse::css_into_node(&mut arena, root_id, css_rules);
    (root_id, arena)
}

/// Flush one rendered frame from the session's `BufWriter`, skipping the
/// syscall entirely when the frame wrote nothing (an unchanged viewport).
fn flush_frame<W: Write>(out: &mut BufWriter<W>) {
    if out.buffer().is_empty() {
        return;
    }
    if let Err(e) = out.flush() {
        log::error!("Failed to flush stdout: {}", e);
    }
}

fn build_css_rules(html: &str, css: &str) -> CssRule {
    let mut html_parser = crate::html::html_parse::Parser::new(html);
    let (_root_id, _arena, embedded_css) = html_parser.parse();

    let combined = if css.is_empty() {
        embedded_css
    } else {
        format!("{}\n{}", embedded_css, css)
    };

    if combined.trim().is_empty() {
        return CssRule::new();
    }

    let mut css_parser = css_parse::Parser::new(&combined);
    css_parser.parse()
}

/// Extract the document `<title>` text from the parsed DOM, if present.
///
/// Browsers use the first `<title>` element and ignore later ones; this walks
/// the arena in document order and returns the concatenated text of the first
/// `<title>` found. Returns `None` when the document has no `<title>` or it
/// contains no non-whitespace text.
fn extract_title(arena: &NodeArena, root_id: NodeId) -> Option<String> {
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        if node.node_type == NodeType::Title {
            let text: String = node
                .children
                .iter()
                .filter_map(|&child_id| match &arena[child_id].node_type {
                    NodeType::Text(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            let text = text.trim();
            return if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            };
        }
        // Reverse so the stack visits children in document order (LIFO).
        stack.extend(node.children.iter().rev().copied());
    }
    None
}

// ============================================================
// Render thread
// ============================================================

/// Per-session state owned by the render thread.
///
/// Holds everything that outlives a single frame: the session document, the
/// stylesheet, the reused canvas, the cached geometry and the scroll bound
/// shared with the input loop.
/// Highlight colour for the page text selection (Tokyo Night selection).
const SELECTION_BG: Color = Color::Rgb {
    r: 0x3b,
    g: 0x42,
    b: 0x61,
};

/// An active page text selection: the two canvas-space endpoints and the
/// shape connecting them (`Flow` follows reading order, `Block` is a
/// rectangle). The highlight and the extractable text are both derived from
/// the same per-row column bands, so what is highlighted is exactly what gets
/// copied.
#[derive(Debug, Clone, Copy)]
struct TextSel {
    mode: SelectionMode,
    anchor: (u32, u32),
    end: (u32, u32),
}

struct RenderState {
    html: Arc<str>,
    css_rules: Arc<CssRule>,
    /// Whether the stylesheet declares any `:hover` rule — when false, moving
    /// the pointer can never change the painted output, so `Hover` commands
    /// are skipped entirely (browser-like no-op).
    has_hover_rules: bool,
    term_layout: Option<TermLayout>,
    /// Terminal title: install the document `<title>` once per session (or
    /// per replaced document). crossterm offers no title getter, so the
    /// previous title is intentionally left untouched on quit.
    title_set: bool,
    /// Cached geometry of the latest layout pass: kept so `RenderCmd::Hover`
    /// can re-style the document for CSS `:hover` rules and repaint without
    /// re-parsing or re-laying out.
    geometry: Option<Arc<Geometry>>,
    /// The node currently under the pointer (for CSS `:hover` cascade on
    /// `RenderArena` re-renders).
    hovered: Option<NodeId>,
    /// Active page text selection (canvas-space endpoints + shape); `None`
    /// while no selection exists.
    text_selection: Option<TextSel>,
    /// Text inside the current selection, refreshed whenever the selection
    /// moves; the input loop reads it on `Ctrl+C` to copy.
    selection_text: Arc<Mutex<String>>,
    max_scroll: Arc<AtomicU32>,
}

impl RenderState {
    /// Repaint one `<input>` field's rectangle with fresh edit state. The
    /// field's layout is width-fixed, so value / caret / focus never change
    /// geometry — only the field's cells are repainted.
    fn handle_input<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        node_id: NodeId,
        value: String,
        cursor: usize,
        focused: bool,
        selection: Option<(usize, usize)>,
    ) {
        let Some(tl) = self.term_layout.as_mut() else {
            return;
        };
        let Some(geo) = self.geometry.as_ref() else {
            return;
        };
        tl.input_states.insert(
            node_id,
            InputRenderState {
                value,
                cursor,
                focused,
                selection,
            },
        );
        let rect = collect_box_rects(&geo.boxes)
            .into_iter()
            .find(|(id, _)| *id == node_id)
            .map(|(_, r)| r);
        match rect {
            Some(r) => tl.repaint_region(&geo.arena, &geo.boxes, r),
            None => tl.repaint_from(&geo.arena, &geo.boxes),
        }
        tl.render(out);
        flush_frame(out);
    }

    /// Parse the current session document and cascade its styles.
    fn parse_and_apply(&self) -> (NodeId, NodeArena) {
        parse_and_apply(&self.html, &self.css_rules)
    }

    /// Re-parse, re-layout, re-paint, publish the geometry and flush a frame.
    ///
    /// Shared by `Redraw` (same document) and `RenderHtml` (new document).
    /// `out` is generic so tests can drive the pipeline against an in-memory
    /// writer.
    fn redraw<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        scroll_y: u32,
        zoom: f32,
    ) {
        let (root_id, arena) = self.parse_and_apply();
        self.layout_and_paint(out, geo_tx, term_map, arena, root_id, scroll_y, zoom, None);
    }

    /// Re-layout / re-paint a supplied DOM arena, publishing geometry.
    ///
    /// Shared tail of [`redraw`](Self::redraw) and
    /// [`render_arena`](Self::render_arena). `arena` must already carry its
    /// cascaded CSS (`css_attr`); the caller handles parsing or re-cascading.
    /// `dirty` carries the nodes JS mutated since the previous frame: when
    /// the re-layout leaves every box rect unchanged, only those nodes'
    /// region is repainted instead of the whole canvas.
    fn layout_and_paint<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        arena: NodeArena,
        root_id: NodeId,
        scroll_y: u32,
        zoom: f32,
        dirty: Option<&DirtyInfo>,
    ) {
        // Reuse the existing canvas when its dimensions still match the
        // terminal. Only allocate a fresh Vec on the first render or after a
        // terminal resize, when the buffer size must change. A resize also
        // invalidates every cached pixel, so it forces the full-repaint path.
        let size_changed = match self.term_layout.as_ref() {
            Some(tl) => tl.columns != term_map.columns || tl.rows != term_map.rows,
            None => true,
        };
        if size_changed {
            self.term_layout = Some(TermLayout::new(term_map, zoom));
        }
        let tl = self
            .term_layout
            .as_mut()
            .expect("TermLayout just allocated above or reused from prior render");

        let boxes = match tl.layout_only(&arena, root_id, term_map) {
            Ok(b) => b,
            Err(e) => {
                log::error!("Layout failed: {}", e);
                return;
            }
        };

        // Install the document `<title>` as the terminal window title once
        // per session. The DOM is re-parsed on every `Redraw` but the title
        // is static, so this only runs on the first successful layout.
        if !self.title_set {
            if let Some(title) = extract_title(&arena, root_id) {
                execute!(stdout(), crossterm::terminal::SetTitle(title)).unwrap_or_else(|e| {
                    log::error!("Failed to set terminal title: {}", e)
                });
            }
            self.title_set = true;
        }

        let geo = Arc::new(Geometry {
            arena,
            boxes,
            root_id,
        });

        // Incremental repaint: when JS only touched a few nodes, the fresh
        // layout usually keeps every box rect identical to the cached one, so
        // only the dirty nodes' region can differ from the previous frame.
        // Repainting just that region (plus the ancestor background fills)
        // avoids clearing and re-painting the whole buffer. Any mismatch — a
        // moved / reflowed box, a hover change, a canvas resize — falls back
        // to the full repaint.
        let region = if !size_changed
            && let Some(dirty) = dirty
            && !dirty.is_empty()
            && let Some(prev) = &self.geometry
        {
            incremental_region(&prev.boxes, &geo.boxes, &geo.arena, &dirty.nodes)
        } else {
            None
        };

        // Publish the layout snapshot: cache it for `:hover` re-styling and
        // forward a copy to the hit-test thread when one is listening (the
        // event system is active).
        self.geometry = Some(Arc::clone(&geo));
        if let Some(tx) = geo_tx
            && let Err(e) = tx.send(Arc::clone(&geo))
        {
            // No hit-test thread is listening (plain `render`), so the
            // snapshot has no consumer — benign.
            log::debug!("Hit-test channel closed: {}", e);
        }

        // Publish the new upper bound of `scroll_y` so the input loop can
        // drop events that would no-op against it. u32::MAX is used as the
        // startup placeholder (no upper bound known yet), and the first
        // successful layout will overwrite it with the real value.
        self.max_scroll.store(tl.max_scroll(), Ordering::Relaxed);

        tl.scroll_y = scroll_y;
        tl.clamp_scroll();

        match region {
            Some(r) => tl.repaint_region(&geo.arena, &geo.boxes, r),
            None => tl.repaint_from(&geo.arena, &geo.boxes),
        }

        // Re-apply the active page selection: the paint pass has no knowledge
        // of it, so any freshly painted canvas needs the highlight overlaid.
        // Flow selections are clipped to the text columns so the highlight
        // never spills over borders / padding / trailing whitespace.
        if let Some(sel) = &self.text_selection {
            let lines = highlight_lines(&geo.arena, &geo.boxes, sel, tl.columns as u32);
            apply_selection_lines(tl, &lines);
        }

        // The incremental renderer emits only the changed cells (and a
        // one-time full clear on the first frame / after resize), so redraws
        // need no per-frame Clear. `flush_frame` is the single I/O per frame,
        // skipped entirely when nothing changed.
        tl.render(out);
        flush_frame(out);
    }

    /// Re-cascade CSS (with the current `:hover`) into a script-engine arena
    /// snapshot and render it without re-parsing HTML.
    ///
    /// Backs `RenderCmd::RenderArena`: JS mutated the live DOM, so the arena
    /// arrives raw (no `css_attr`) and must be cascaded first. `dirty` lists
    /// the mutated nodes for the incremental repaint; `hover_changed` is true
    /// when the `:hover` target moved, which invalidates the incremental path
    /// (the highlight may live anywhere in the tree).
    fn render_arena<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        arena: &NodeArena,
        root_id: NodeId,
        scroll_y: u32,
        zoom: f32,
        dirty: DirtyInfo,
        hover_changed: bool,
    ) {
        let mut arena = arena.clone();
        reset_css_attr(&mut arena, root_id);
        css_into_node_with_hover(&mut arena, root_id, &self.css_rules, self.hovered);
        let dirty = if hover_changed { None } else { Some(&dirty) };
        self.layout_and_paint(out, geo_tx, term_map, arena, root_id, scroll_y, zoom, dirty);
    }

    /// Replace the session document and fully re-render it, keeping the
    /// current scroll position / zoom.
    ///
    /// Backs `RenderCmd::RenderHtml` — the dynamic-content entry point that
    /// lets event handlers rebuild the document (e.g. a counter that
    /// re-renders on every click).
    fn render_html<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        html: Arc<str>,
    ) {
        let (scroll_y, zoom) = match &self.term_layout {
            Some(tl) => (tl.scroll_y, tl.zoom),
            None => (0, 1.0),
        };
        self.html = html;
        // The new document may carry a different `<title>`.
        self.title_set = false;
        self.redraw(out, geo_tx, term_map, scroll_y, zoom);
    }
}

/// Compute the smallest rectangle whose repaint covers every change caused by
/// the dirty nodes, or `None` when the incremental path must be abandoned.
///
/// A DOM node paints inside the box of its nearest box-carrying ancestor
/// (inline elements fold into the enclosing block's runs), so each dirty node
/// is mapped up the DOM to the first ancestor that produced a `LayoutBox`.
/// That box must exist in both the cached and the fresh layout with an
/// identical rect — otherwise the change moved geometry (e.g. wrapping or a
/// structural mutation) and only a full repaint is safe. The union of the
/// matched boxes' rects is the repaint region.
fn incremental_region(
    old_boxes: &[LayoutBox],
    new_boxes: &[LayoutBox],
    arena: &NodeArena,
    dirty: &[NodeId],
) -> Option<Rect> {
    let old_rects = collect_box_rects(old_boxes);
    let new_rects = collect_box_rects(new_boxes);
    let mut region: Option<Rect> = None;
    for &node_id in dirty {
        let mut found = false;
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            let rects: Vec<Rect> = new_rects
                .iter()
                .filter(|(n, _)| *n == id)
                .map(|(_, r)| *r)
                .collect();
            if !rects.is_empty() {
                found = true;
                for rect in rects {
                    let old_ok = old_rects.iter().any(|(n, r)| *n == id && *r == rect);
                    if !old_ok {
                        return None;
                    }
                    region = Some(match region {
                        Some(r) => union_rect(r, rect),
                        None => rect,
                    });
                }
                break;
            }
            cur = arena.get(id).and_then(|n| n.parent);
        }
        if !found {
            return None;
        }
    }
    region
}

/// Union of two canvas rectangles (absolute coordinates).
fn union_rect(a: Rect, b: Rect) -> Rect {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = a.x.saturating_add(a.width).max(b.x.saturating_add(b.width));
    let bottom = a
        .y
        .saturating_add(a.height)
        .max(b.y.saturating_add(b.height));
    Rect::new(left, top, right - left, bottom - top)
}

/// The per-row column bands a selection covers — the single source of truth
/// for both the highlight and the extracted text. Each entry is
/// `(row, col_start, col_end_exclusive)`.
///
/// Flow (left-drag) follows reading order: the anchor row runs from the
/// anchor column to the right edge, the end row from the left edge to the end
/// column, and every row in between is full-width. Block (right-drag) is a
/// plain rectangle.
fn selection_lines(sel: &TextSel, cols: u32) -> Vec<(u32, u32, u32)> {
    let (ax, ay) = sel.anchor;
    let (ex, ey) = sel.end;
    match sel.mode {
        SelectionMode::Block => {
            let x1 = ax.min(ex);
            let x2 = ax.max(ex).saturating_add(1);
            let y1 = ay.min(ey);
            let y2 = ay.max(ey).saturating_add(1);
            (y1..y2).map(|y| (y, x1, x2)).collect()
        }
        SelectionMode::Flow => {
            if ay == ey {
                vec![(ay, ax.min(ex), ax.max(ex).saturating_add(1))]
            } else if ay < ey {
                let mut lines = vec![(ay, ax, cols)];
                for y in ay.saturating_add(1)..ey {
                    lines.push((y, 0, cols));
                }
                lines.push((ey, 0, ex.saturating_add(1)));
                lines
            } else {
                // Dragged upward: the end point sits above the anchor.
                let mut lines = vec![(ey, ex, cols)];
                for y in ey.saturating_add(1)..ay {
                    lines.push((y, 0, cols));
                }
                lines.push((ay, 0, ax.saturating_add(1)));
                lines
            }
        }
    }
}

/// Bounding rectangle of a set of line bands (used to pick the repaint
/// region; the bands themselves drive the actual highlighting).
fn lines_bounds(lines: &[(u32, u32, u32)]) -> Option<Rect> {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    for &(y, xs, xe) in lines {
        min_y = min_y.min(y);
        min_x = min_x.min(xs);
        max_y = max_y.max(y);
        max_x = max_x.max(xe);
    }
    if max_y < min_y {
        return None;
    }
    Some(Rect::new(
        min_x as usize,
        min_y as usize,
        (max_x - min_x) as usize,
        (max_y - min_y + 1) as usize,
    ))
}

/// The line bands a selection actually highlights: flow selections are
/// clipped to the columns the text occupies (so the highlight never washes
/// over a block's border, padding, indentation or trailing whitespace), while
/// block selections keep the dragged rectangle.
fn highlight_lines(arena: &NodeArena, boxes: &[LayoutBox], sel: &TextSel, cols: u32) -> Vec<(u32, u32, u32)> {
    let raw = selection_lines(sel, cols);
    match sel.mode {
        SelectionMode::Flow => {
            crate::layout::selection_text_lines(arena, boxes, &raw)
        }
        SelectionMode::Block => raw,
    }
}

/// Overlay the page-selection highlight on every cell inside the line bands.
///
/// The paint phase knows nothing about the selection, so this is applied after
/// each repaint. Filler cells (empty word, the second half of a wide glyph)
/// get the background too so the highlight reads as a continuous band; the
/// diff renderer skips them, so only glyph-bearing cells change the output.
fn apply_selection_lines(tl: &mut TermLayout, lines: &[(u32, u32, u32)]) {
    let cols = tl.columns as usize;
    let max = tl.layout.len();
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
            tl.layout[idx].bcolor = Some(SELECTION_BG);
        }
    }
}

/// Extract the text covered by the selection's line bands, row by row.
///
/// Filler cells of wide glyphs are skipped (their glyph lives in the first
/// column), and trailing spaces are trimmed per row so a flow selection of a
/// paragraph does not copy its right-edge padding.
fn extract_selection(tl: &TermLayout, lines: &[(u32, u32, u32)]) -> String {
    let cols = tl.columns as usize;
    let max = tl.layout.len();
    let mut out = String::new();
    for &(y, xs, xe) in lines {
        let row_start = (y as usize).saturating_mul(cols);
        let xe = (xe as usize).min(cols);
        let mut line = String::new();
        for x in (xs as usize)..xe {
            let idx = row_start.saturating_add(x);
            if idx >= max {
                break;
            }
            let word = &tl.layout[idx].word;
            if !word.is_empty() {
                line.push_str(word);
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Flatten the box forest into `(node_id, rect)` pairs.
fn collect_box_rects(boxes: &[LayoutBox]) -> Vec<(NodeId, Rect)> {
    fn walk(boxes: &[LayoutBox], out: &mut Vec<(NodeId, Rect)>) {
        for b in boxes {
            out.push((b.node_id, b.rect));
            walk(&b.children, out);
        }
    }
    let mut out = Vec::new();
    walk(boxes, &mut out);
    out
}

/// Union of every box rect belonging to `node_id` or any of its ancestors
/// that has a box.
///
/// `:hover` rules may match any element on the hovered node's ancestor chain
/// (`#panel:hover` matches while a descendant is hovered), so repainting this
/// union covers every cell the hover state can change. Used by the `Hover`
/// command to repaint only the affected region instead of the whole canvas.
fn hover_affected_region(
    boxes: &[LayoutBox],
    arena: &NodeArena,
    node_id: Option<NodeId>,
) -> Option<Rect> {
    let mut region: Option<Rect> = None;
    let mut cur = node_id;
    while let Some(id) = cur {
        let mut stack = vec![boxes];
        while let Some(list) = stack.pop() {
            for b in list {
                if b.node_id == id {
                    region = Some(match region {
                        Some(r) => union_rect(r, b.rect),
                        None => b.rect,
                    });
                }
                stack.push(&b.children);
            }
        }
        cur = arena.get(id).and_then(|n| n.parent);
    }
    region
}

/// True when the stylesheet contains any `:hover` rule — the only case where
/// moving the pointer can change the painted output.
fn css_has_hover_rules(css_rules: &CssRule) -> bool {
    css_rules.0.keys().any(|sel| {
        sel.compounds.iter().any(|compound| {
            compound
                .iter()
                .any(|s| matches!(s, Selector::PseudoClass(PseudoClass::Hover)))
        })
    })
}

fn render_thread(
    rx: mpsc::Receiver<RenderCmd>,
    html: Arc<str>,
    css_rules: Arc<CssRule>,
    max_scroll: Arc<AtomicU32>,
    geo_tx: Option<mpsc::Sender<Arc<Geometry>>>,
    selection_text: Arc<Mutex<String>>,
) {
    // One persistent write buffer for the whole session: every frame writes
    // into it and is flushed exactly once, so a frame costs a single write
    // syscall instead of one per render call.
    let mut out = BufWriter::new(stdout());
    let has_hover_rules = css_has_hover_rules(&css_rules);
    let mut state = RenderState {
        html,
        css_rules,
        has_hover_rules,
        term_layout: None,
        title_set: false,
        geometry: None,
        hovered: None,
        text_selection: None,
        selection_text,
        max_scroll,
    };

    loop {
        let mut cmd = match rx.recv() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Render channel disconnected: {}", e);
                break;
            }
        };
        // `try_recv` coalescing keeps only the last command — but input
        // repaints must all run (a focus switch queues blur + focus for two
        // different fields, and dropping either leaves a stale caret). The
        // first command of the batch is no exception: when it is a
        // `RenderInput` (e.g. a blur repaint) and a later non-input command
        // arrives (typically the `RenderArena` triggered by the blur
        // handler's DOM write), the replacement below would silently drop it
        // — `input_states` never updates and the field keeps its focused
        // caret even though the input loop already blurred it.
        let mut pending_inputs: Vec<(NodeId, String, usize, bool, Option<(usize, usize)>)> =
            Vec::new();
        while let Ok(next) = rx.try_recv() {
            match next {
                RenderCmd::RenderInput {
                    node_id,
                    value,
                    cursor,
                    focused,
                    selection,
                } => pending_inputs.push((node_id, value, cursor, focused, selection)),
                other => {
                    // The command being coalesced away may itself be an input
                    // repaint — rescue it into the pending pass (it arrived
                    // first, so it goes to the front of the queue).
                    if let RenderCmd::RenderInput {
                        node_id,
                        value,
                        cursor,
                        focused,
                        selection,
                    } = std::mem::replace(&mut cmd, other)
                    {
                        pending_inputs.insert(0, (node_id, value, cursor, focused, selection));
                    }
                    if matches!(cmd, RenderCmd::Quit) {
                        break;
                    }
                }
            }
        }
        for (node_id, value, cursor, focused, selection) in pending_inputs {
            state.handle_input(&mut out, node_id, value, cursor, focused, selection);
        }

        match cmd {
            RenderCmd::Quit => break,

            RenderCmd::Rerender { scroll_y } => {
                let Some(tl) = state.term_layout.as_mut() else {
                    continue;
                };
                let prev = tl.scroll_y;
                tl.scroll_y = scroll_y;
                tl.clamp_scroll();
                // Defense in depth: drop rerenders whose resolved scroll
                // position is unchanged after clamping. Most such events are
                // already filtered in the input thread, but if a `Rerender`
                // slips past (e.g. races between a Redraw updating the
                // max_scroll ceiling and a queued scroll event) we still
                // avoid the diff pass.
                if tl.scroll_y == prev {
                    continue;
                }
                // The incremental renderer diffs the shifted viewport against
                // the previous frame, so scrolling needs no full-screen Clear.
                tl.render(&mut out);
                flush_frame(&mut out);
            }

            RenderCmd::Redraw { scroll_y, zoom } => {
                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        continue;
                    }
                };
                state.redraw(&mut out, &geo_tx, &term_map, scroll_y, zoom);
            }

            RenderCmd::RenderArena {
                arena,
                root_id,
                scroll_y,
                zoom,
                hovered,
                dirty,
            } => {
                // A hover change re-styles the whole tree (the `:hover`
                // highlight may sit anywhere), so the incremental repaint is
                // only valid when the hover target is unchanged.
                let hover_changed = state.hovered != hovered;
                state.hovered = hovered;
                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        continue;
                    }
                };
                state.render_arena(
                    &mut out,
                    &geo_tx,
                    &term_map,
                    &arena,
                    root_id,
                    scroll_y,
                    zoom,
                    dirty,
                    hover_changed,
                );
            }

            RenderCmd::RenderHtml { html } => {
                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        continue;
                    }
                };
                state.render_html(&mut out, &geo_tx, &term_map, html);
            }

            RenderCmd::Hover { node_id } => {
                let prev = state.hovered;
                state.hovered = node_id;
                // Without any `:hover` rule the cascade is identical for every
                // pointer position, so nothing can change on screen — skip the
                // repaint entirely (browser behaviour: moving the pointer with
                // no `:hover` styles produces no drawing).
                if !state.has_hover_rules {
                    continue;
                }
                let Some(tl) = state.term_layout.as_mut() else {
                    continue;
                };
                let Some(geo) = state.geometry.as_ref() else {
                    continue;
                };
                // Re-run the cascade with the hovered node so `:hover` rules
                // apply. The arena is cloned — the shared snapshot stays
                // immutable for the hit-test thread, and a slotmap clone keeps
                // the same keys, so the cached boxes remain valid (hover only
                // changes colours / backgrounds, never geometry).
                let mut arena = geo.arena.clone();
                reset_css_attr(&mut arena, geo.root_id);
                css_into_node_with_hover(&mut arena, geo.root_id, &state.css_rules, node_id);
                // Incremental repaint: only the union of the old and new hover
                // regions can differ from the previous frame (the old
                // highlight is cleared, the new one painted). Repainting this
                // small region instead of resetting the whole canvas keeps
                // hover tracking responsive while the pointer moves.
                let region = match (
                    hover_affected_region(&geo.boxes, &arena, prev),
                    hover_affected_region(&geo.boxes, &arena, node_id),
                ) {
                    (Some(a), Some(b)) => Some(union_rect(a, b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };
                match region {
                    Some(r) => tl.repaint_region(&arena, &geo.boxes, r),
                    None => tl.repaint_from(&arena, &geo.boxes),
                }
                // The repaint reset the canvas (or part of it); the active
                // selection highlight must be overlaid again.
                if let Some(sel) = &state.text_selection {
                    let lines = highlight_lines(&geo.arena, &geo.boxes, sel, tl.columns as u32);
                    apply_selection_lines(tl, &lines);
                }
                tl.render(&mut out);
                flush_frame(&mut out);
            }

            RenderCmd::TextSelection { anchor, end, mode } => {
                let new_sel = match (anchor, end) {
                    (Some(a), Some(b)) => Some(TextSel {
                        mode,
                        anchor: a,
                        end: b,
                    }),
                    _ => None,
                };
                let prev = state.text_selection;
                state.text_selection = new_sel;
                let Some(tl) = state.term_layout.as_mut() else {
                    continue;
                };
                let Some(geo) = state.geometry.as_ref() else {
                    continue;
                };
                let cols = tl.columns as u32;
                // Highlight bands are the actual cells the selection covers:
                // flow selections clip to the text columns, block selections
                // keep the rectangle.
                let prev_lines = match &prev {
                    Some(n) => highlight_lines(&geo.arena, &geo.boxes, n, cols),
                    None => Vec::new(),
                };
                let new_lines = match &new_sel {
                    Some(n) => highlight_lines(&geo.arena, &geo.boxes, n, cols),
                    None => Vec::new(),
                };
                // Repaint the union of the old and new selection bounds: the
                // old highlight must be cleared, the new one painted.
                let region = match (lines_bounds(&prev_lines), lines_bounds(&new_lines)) {
                    (Some(a), Some(b)) => Some(union_rect(a, b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };
                if let Some(r) = region {
                    tl.repaint_region(&geo.arena, &geo.boxes, r);
                    apply_selection_lines(tl, &new_lines);
                }
                tl.render(&mut out);
                flush_frame(&mut out);
                // Refresh the copyable text of the selection (or clear it).
                // Flow selections are rebuilt from the layout's inline runs so
                // wrapped lines inside one block stay continuous (browser
                // behaviour); block selections keep the canvas extraction.
                let raw_lines = match &new_sel {
                    Some(n) => selection_lines(n, cols),
                    None => Vec::new(),
                };
                let text = match &new_sel {
                    Some(n) if n.mode == SelectionMode::Flow => {
                        crate::layout::extract_flow_text(&geo.arena, &geo.boxes, &raw_lines)
                    }
                    Some(_) => extract_selection(tl, &new_lines),
                    None => String::new(),
                };
                if let Ok(mut guard) = state.selection_text.lock() {
                    *guard = text;
                }
            }

            RenderCmd::RenderInput {
                node_id,
                value,
                cursor,
                focused,
                selection,
            } => {
                state.handle_input(&mut out, node_id, value, cursor, focused, selection);
            }
        }
    }
}

/// Worker of the dedicated hit-test thread.
///
/// Consumes geometry snapshots published by the render thread and replaces
/// the shared hit map with a freshly built region list. Exits when the render
/// thread drops the channel (i.e. on quit).
fn hit_test_thread(rx: mpsc::Receiver<Arc<Geometry>>, hit_map: Arc<Mutex<Arc<HitMap>>>) {
    while let Ok(geo) = rx.recv() {
        let map = Arc::new(HitMap::build(&geo));
        match hit_map.lock() {
            Ok(mut guard) => *guard = map,
            Err(e) => log::error!("Hit map lock poisoned: {}", e),
        }
    }
    log::debug!("Hit-test thread exiting");
}

// debug panic hook
// this fun only used in debug to hook render panic
pub(crate) fn run(html: &str, css: &str, bindings: Option<EventBindings>) {
    // install debug hook
    #[cfg(debug_assertions)]
    install_debug_panic_hook();
    
    if let Err(e) = enable_raw_mode() {
        log::error!("Failed to enable raw mode: {}", e);
        return;
    }
    if let Err(e) = execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, Hide) {
        log::error!("Failed to enter alternate screen: {}", e);
        if let Err(e2) = disable_raw_mode() {
            log::error!("Failed to disable raw mode during cleanup: {}", e2);
        }
        return;
    }

    let css_rules = Arc::new(build_css_rules(html, css));

    let html: Arc<str> = Arc::from(html);
    // The script engine owns the live DOM when events are enabled; it parses
    // the document itself (arena + `<script>` blocks) and reports dirty flags
    // when JS mutates the DOM.
    let mut engine: Option<ScriptEngine> = None;
    if let Some(bindings) = bindings {
        engine = match ScriptEngine::new(html.as_ref(), bindings) {
            Ok(e) => Some(e),
            Err(err) => {
                log::error!("Failed to initialize script engine: {err}");
                None
            }
        };
    }

    // Shared scroll upper bound — initial value `u32::MAX` means "not yet
    // known" (the first layout hasn't run); the render thread overwrites it
    // on every successful render with the real `tl.max_scroll()`.
    let max_scroll = Arc::new(AtomicU32::new(u32::MAX));

    // Geometry snapshot channel (render thread → hit-test thread) and the
    // published hit map (hit-test thread → input loop). The hit-test thread
    // is only spawned when event handling is requested.
    let (geo_tx, geo_rx) = mpsc::channel::<Arc<Geometry>>();
    let hit_map: Arc<Mutex<Arc<HitMap>>> = Arc::new(Mutex::new(Arc::new(HitMap::new())));
    if engine.is_some() {
        let hit_map_thread = Arc::clone(&hit_map);
        std::thread::spawn(move || hit_test_thread(geo_rx, hit_map_thread));
    }

    // Text of the current page selection, refreshed by the render thread and
    // read by the input loop on `Ctrl+C` (shared through `Arc<Mutex<_>>`).
    let selection_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    let (tx, rx) = mpsc::channel::<RenderCmd>();
    let render_handle = std::thread::spawn({
        let html_clone = Arc::clone(&html);
        let css_clone = Arc::clone(&css_rules);
        let max_scroll_clone = Arc::clone(&max_scroll);
        let selection_text_clone = Arc::clone(&selection_text);
        let geo_tx = engine.is_some().then_some(geo_tx);
        move || {
            render_thread(
                rx,
                html_clone,
                css_clone,
                max_scroll_clone,
                geo_tx,
                selection_text_clone,
            );
        }
    });

    let mut state = InputState::new();
    state.selection_text = Arc::clone(&selection_text);
    let mut rows: u16 = match TermMap::get_ter_map() {
        Ok(m) => m.rows,
        Err(e) => {
            log::error!("Failed to get terminal size: {}", e);
            24
        }
    };

    // First frame: the engine's live arena (scripts already ran and may have
    // mutated the DOM) or a plain parse-based redraw.
    let first_cmd = match engine.as_mut() {
        Some(eng) => RenderCmd::RenderArena {
            arena: Arc::new(eng.arena()),
            root_id: eng.root_id(),
            scroll_y: state.scroll_y,
            zoom: state.zoom,
            hovered: None,
            dirty: DirtyInfo::default(),
        },
        None => RenderCmd::Redraw {
            scroll_y: state.scroll_y,
            zoom: state.zoom,
        },
    };
    if let Err(e) = tx.send(first_cmd) {
        log::error!("Failed to send initial render: {}", e);
        if let Err(e2) = tx.send(RenderCmd::Quit) {
            log::error!("Failed to send Quit during cleanup: {}", e2);
        }
    }

    // Periodic auto action (see `EventBindings::auto`): cloned once so the
    // loop can read it while mutably borrowing `engine` for dispatch.
    let auto: Option<AutoRefresh> = engine.as_ref().and_then(|e| e.auto());
    let mut last_auto = std::time::Instant::now();

    let mut quit = false;
    while !quit {
        // Detect a dead render thread (e.g. from a panic) promptly. Rust's
        // spawned-thread panic handler swallows the panic and kills just the
        // worker, so without this check the main loop would keep polling
        // events, fail to send, and only exit when the user presses a key.
        if render_handle.is_finished() {
            log::error!("Render thread has terminated unexpectedly");
            break;
        }

        let mut cmds = if engine.is_some() {
            let map = match hit_map.lock() {
                Ok(guard) => Arc::clone(&guard),
                Err(e) => {
                    log::error!("Hit map lock poisoned: {}", e);
                    break;
                }
            };
            trigger_refresh(
                &mut state,
                rows,
                max_scroll.load(Ordering::Relaxed),
                Some(&map),
                engine.as_mut(),
                Some(Duration::from_millis(100)),
            )
        } else {
            trigger_refresh(
                &mut state,
                rows,
                max_scroll.load(Ordering::Relaxed),
                None,
                None,
                Some(Duration::from_millis(100)),
            )
        };

        // Periodic auto action: when the interval has elapsed, invoke the
        // configured JS global function (see `ScriptEngine::invoke_action`)
        // and forward its render commands alongside the input events.
        if let Some(auto) = &auto {
            let now = std::time::Instant::now();
            if now.duration_since(last_auto) >= auto.every
                && let Some(engine) = engine.as_mut()
            {
                last_auto = now;
                cmds.extend(engine.invoke_action(&auto.action));
            }
        }

        // JS mutations of the live DOM need a re-render from the engine's
        // arena (no HTML re-parse). The hover target is folded in so the
        // `RenderArena` cascade re-applies the current `:hover` highlight
        // without a separate `Hover` command. The mutated node ids ride along
        // so the render thread can repaint just those regions.
        if let Some(engine) = engine.as_mut() {
            let dirty = engine.take_dirty();
            if !dirty.is_empty() {
                cmds.push(RenderCmd::RenderArena {
                    arena: Arc::new(engine.arena()),
                    root_id: engine.root_id(),
                    scroll_y: state.scroll_y,
                    zoom: state.zoom,
                    hovered: state.hover.current().map(|r| r.node_id),
                    dirty,
                });
            }
        }

        // Coalesce: when a full `RenderArena` is pending, drop any queued
        // `Hover` commands — the render thread's `try_recv` would otherwise
        // drop them anyway (it keeps only the last command), losing the
        // highlight, or the duplicate repaint would be wasted work. The hover
        // target is already carried by `RenderArena.hovered`.
        let cmds = {
            let mut merged: Vec<RenderCmd> = Vec::with_capacity(cmds.len());
            let mut arena_pending = false;
            for cmd in cmds {
                match cmd {
                    RenderCmd::RenderArena { .. } => {
                        arena_pending = true;
                        merged.push(cmd);
                    }
                    RenderCmd::Hover { .. } if arena_pending => {
                        // Dropped: the arena render re-cascades with `hovered`.
                    }
                    other => merged.push(other),
                }
            }
            merged
        };

        for cmd in cmds {
            if matches!(cmd, RenderCmd::Quit) {
                if let Err(e) = tx.send(RenderCmd::Quit) {
                    log::error!("Failed to send Quit: {}", e);
                }
                quit = true;
                break;
            }
            // With the script engine active, the engine's live arena is the
            // source of truth: convert parse-based `Redraw` (zoom / resize)
            // into a `RenderArena` so JS DOM mutations survive re-renders.
            let cmd = match (cmd, engine.as_mut()) {
                (
                    RenderCmd::Redraw { scroll_y, zoom },
                    Some(engine),
                ) => RenderCmd::RenderArena {
                    arena: Arc::new(engine.arena()),
                    root_id: engine.root_id(),
                    scroll_y,
                    zoom,
                    hovered: state.hover.current().map(|r| r.node_id),
                    dirty: DirtyInfo::default(),
                },
                (other, _) => other,
            };
            if let RenderCmd::Redraw { .. } = &cmd
                && let Ok(map) = TermMap::get_ter_map()
            {
                rows = map.rows;
            }
            if let Err(e) = tx.send(cmd) {
                log::error!("Failed to send render command: {}", e);
                quit = true;
                break;
            }
        }
    }

    if let Err(payload) = render_handle.join() {
        log::error!("Render thread panicked: {}", panic_message(payload.as_ref()));
    }

    if let Err(e) = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, Show) {
        log::error!("Failed to leave alternate screen: {}", e);
    }
    if let Err(e) = disable_raw_mode() {
        log::error!("Failed to disable raw mode: {}", e);
    }
}
#[cfg(debug_assertions)]
use std::panic;
#[cfg(debug_assertions)]
use std::sync::Once;
#[cfg(debug_assertions)]
use std::backtrace::Backtrace;

#[cfg(debug_assertions)]
fn install_debug_panic_hook() {
    static INSTALL_HOOK: Once = Once::new();

    INSTALL_HOOK.call_once(|| {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            default_hook(info);

            let message = if let Some(message) = info.payload().downcast_ref::<&str>() {
                *message
            } else if let Some(message) = info.payload().downcast_ref::<String>() {
                message.as_str()
            } else {
                "non-string panic payload"
            };
            let location = info
                .location()
                .map(|location| {
                    format!(
                        "{}:{}:{}",
                        location.file(),
                        location.line(),
                        location.column()
                    )
                })
                .unwrap_or_else(|| "unknown location".to_owned());
            let thread = std::thread::current();
            let thread_name = thread.name().unwrap_or("unnamed");
            let backtrace = Backtrace::force_capture();

            log::error!(
                "Thread '{thread_name}' panicked at {location}: {message}\nBacktrace:\n{backtrace}"
            );
        }));
    });
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.as_str()
    } else {
        "non-string panic payload"
    }
}

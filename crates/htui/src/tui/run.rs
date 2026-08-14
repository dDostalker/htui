//! Terminal engine driver — the `render` / `render_with_events` entry-point
//! machinery.
//!
//! Owns the render thread (HTML → layout → paint → ANSI diff output), the
//! hit-test thread (geometry snapshots → shared `HitMap`), the incremental
//! region-repaint helpers, and the input loop that translates keys / mouse
//! events into `RenderCmd`s.

use std::io::{BufWriter, Write, stdout};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::{
    cursor::{Hide, Show},
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    style::Color,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
    },
};

use crate::{
    events::{EventBindings, Geometry, HitMap},
    html::css_parse::{
        self, CssRule, DynamicState, PseudoClass, PseudoClassSet, css_into_node_with_state,
        css_pseudo_rule_set, reset_css_attr, selector_chain_matches_with_state,
    },
    layout::layout::{MAX_TERM_COLS, MAX_TERM_ROWS, TermMap},
    layout::layoutbox::geometry::{
        SelectRow, overlay_options, select_item_indices, select_overlay_rect, select_rows,
    },
    layout::{LayoutBox, Rect},
    script::{ScriptMsg, ScriptResponse, run_script_thread},
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::{
        Block, DirtyInfo, FontStyle, InputRenderState, InputState, KEY_GATE_TIMEOUT, RenderCmd,
        SelectionMode, TermLayout, apply_focus_requests, char_width, reapply_console_modes,
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

/// Minimum time between two render-thread frames.
///
/// A fast wheel scroll can queue a full-viewport diff per tick; without a cap
/// the render thread would try to draw at the event rate, saturate the
/// terminal's output pipe and stall on the write (the console can't drain
/// that fast), which looks exactly like an app freeze. Waiting out the gap
/// lets bursts coalesce into fewer, larger frames and bounds the output rate
/// at ~60 fps. 16 ms is imperceptible to interactive input.
const MIN_FRAME_GAP: Duration = Duration::from_millis(16);

fn combined_css_source(html: &str, css: &str) -> String {
    let mut html_parser = crate::html::html_parse::Parser::new(html);
    let (_root_id, _arena, embedded_css) = html_parser.parse();

    if css.is_empty() {
        embedded_css
    } else {
        format!("{}\n{}", embedded_css, css)
    }
}

/// Parse a full CSS source string into rules (empty source → empty rules).
fn parse_css_source(source: &str) -> CssRule {
    if source.trim().is_empty() {
        return CssRule::new();
    }
    let mut css_parser = css_parse::Parser::new(source);
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
                    NodeType::Text => arena[child_id].data.as_deref(),
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

/// Dropdown overlay background (Tokyo Night panel).
const OVERLAY_BG: Color = Color::Rgb {
    r: 0x24,
    g: 0x28,
    b: 0x3b,
};

/// Overlay text colour (Tokyo Night foreground).
const OVERLAY_FG: Color = Color::Rgb {
    r: 0xc0,
    g: 0xca,
    b: 0xf5,
};

/// Foreground of `<optgroup>` header rows — dimmer than the options so a
/// group label reads as a non-option heading (clicking it still picks the
/// group's first option).
const GROUP_FG: Color = Color::Rgb {
    r: 0x7a,
    g: 0x83,
    b: 0xa8,
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

/// An open `<select>` dropdown overlay: the field rect, the option list and
/// the highlighted row. Painted after every repaint (like the text
/// selection) so it survives layout / hover passes.
struct OpenSelect {
    node_id: NodeId,
    /// Overlay rectangle (canvas coordinates, directly below the field).
    rect: Rect,
    /// Flat option list (`value`, `label`, optional group) — the label source
    /// for the row painter.
    options: Vec<crate::layout::layoutbox::geometry::SelectOption>,
    /// Dropdown rows (group headers interleaved with option items).
    rows: Vec<crate::layout::layoutbox::geometry::SelectRow>,
    /// Option index highlighted by the keyboard cursor.
    highlight: usize,
}

impl OpenSelect {
    /// The row index holding the highlighted option (first row when the
    /// highlight is stale / absent).
    fn highlight_row(&self) -> usize {
        self.rows
            .iter()
            .position(|r| matches!(r, SelectRow::Item(i) if *i == self.highlight))
            .unwrap_or(0)
    }
}

struct RenderState {
    html: Arc<str>,
    /// Combined initial stylesheet source (document-embedded + session CSS),
    /// kept so `innerHTML`-injected CSS can be appended and re-parsed.
    base_css: Arc<str>,
    /// CSS injected via `innerHTML` `<style>` elements, appended to the
    /// session stylesheet incrementally.
    extra_css: String,
    css_rules: Arc<CssRule>,
    /// The dynamic pseudo-classes the stylesheet uses, computed once at rule
    /// build time; `Hover` / `Focus` commands test membership in O(1) and are
    /// skipped entirely when the stylesheet has no matching rule (browser-like
    /// no-op).
    pseudo_rules: PseudoClassSet,
    term_layout: Option<TermLayout>,
    /// Terminal title: install the document `<title>` once per session (or
    /// per replaced document). crossterm offers no title getter, so the
    /// previous title is intentionally left untouched on quit.
    title_set: bool,
    /// The session interaction state driving the dynamic pseudo-classes:
    /// `hovered` (under the pointer) and `focused` (focused text field),
    /// mirrored from the input loop / script thread for `RenderArena` and
    /// `Hover` / `Focus` re-cascades.
    dynamic: DynamicState,
    /// Cached geometry of the latest layout pass: kept so `RenderCmd::Hover`
    /// / `RenderCmd::Focus` can re-style the document for CSS `:hover` /
    /// `:focus` rules and repaint without re-parsing or re-laying out.
    geometry: Option<Arc<Geometry>>,
    /// Active page text selection (canvas-space endpoints + shape); `None`
    /// while no selection exists.
    text_selection: Option<TextSel>,
    /// Text inside the current selection, refreshed whenever the selection
    /// moves; the input loop reads it on `Ctrl+C` to copy.
    selection_text: Arc<Mutex<String>>,
    /// The `<select>` dropdown overlay currently painted (`None` = none).
    open_select: Option<OpenSelect>,
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

    /// Handle one `<select>` dropdown command (open / close / move).
    ///
    /// Runs both from the coalescing `match` (a lone command) and from the
    /// pending pass, so `SelectClose` cannot be swallowed by a following
    /// `RenderArena` (which would re-draw the closed overlay on its full
    /// repaint) and `SelectMove` cannot be swallowed by a `Hover`.
    fn handle_select<W: Write>(&mut self, out: &mut BufWriter<W>, cmd: RenderCmd) {
        match cmd {
            RenderCmd::SelectOpen { node_id, filter } => {
                let Some(geo) = self.geometry.as_ref() else {
                    return;
                };
                // A previous dropdown may still be painted (its `SelectClose`
                // could have been coalesced away when opening another select):
                // clear the old overlay before painting the new one.
                if let Some(prev) = self.open_select.take()
                    && prev.node_id != node_id
                    && let Some(tl) = self.term_layout.as_mut()
                {
                    tl.open_select = None;
                    let prev_select = collect_box_rects(&geo.boxes)
                        .into_iter()
                        .find(|(id, _)| *id == prev.node_id)
                        .map(|(_, r)| r)
                        .unwrap_or(prev.rect);
                    tl.repaint_region(&geo.arena, &geo.boxes, union_rect(prev_select, prev.rect));
                }
                let options = overlay_options(&geo.arena, node_id, filter.as_deref());
                let rows = select_rows(&options);
                let selected = match filter {
                    // A datalist autocomplete starts at the first suggestion.
                    Some(_) => 0,
                    None => {
                        let value = geo
                            .arena
                            .get(node_id)
                            .and_then(|n| n.attributes.get("value"))
                            .cloned()
                            .unwrap_or_default();
                        options.iter().position(|o| o.value == value).unwrap_or(0)
                    }
                };
                let Some(select_rect) = collect_box_rects(&geo.boxes)
                    .into_iter()
                    .find(|(id, _)| *id == node_id)
                    .map(|(_, r)| r)
                else {
                    return;
                };
                let rect = select_overlay_rect(&options, &rows, select_rect);
                self.open_select = Some(OpenSelect {
                    node_id,
                    rect,
                    options,
                    rows,
                    highlight: selected,
                });
                let Some(tl) = self.term_layout.as_mut() else {
                    return;
                };
                tl.open_select = Some(node_id);
                tl.repaint_region(&geo.arena, &geo.boxes, union_rect(select_rect, rect));
                if let Some(open) = &self.open_select {
                    apply_select_overlay(tl, open);
                }
                tl.render(out);
                flush_frame(out);
            }

            RenderCmd::SelectClose { .. } => {
                let Some(open) = self.open_select.take() else {
                    return;
                };
                let Some(tl) = self.term_layout.as_mut() else {
                    return;
                };
                tl.open_select = None;
                let Some(geo) = self.geometry.as_ref() else {
                    return;
                };
                let select_rect = collect_box_rects(&geo.boxes)
                    .into_iter()
                    .find(|(id, _)| *id == open.node_id)
                    .map(|(_, r)| r)
                    .unwrap_or(open.rect);
                tl.repaint_region(&geo.arena, &geo.boxes, union_rect(select_rect, open.rect));
                tl.render(out);
                flush_frame(out);
            }

            RenderCmd::SelectMove { node_id, highlight } => {
                let Some(open) = self.open_select.as_mut() else {
                    return;
                };
                if open.node_id != node_id {
                    return;
                }
                let item_count = select_item_indices(&open.rows).len();
                open.highlight = highlight.min(item_count.saturating_sub(1));
                let Some(tl) = self.term_layout.as_mut() else {
                    return;
                };
                let Some(geo) = self.geometry.as_ref() else {
                    return;
                };
                tl.repaint_region(&geo.arena, &geo.boxes, open.rect);
                apply_select_overlay(tl, open);
                tl.render(out);
                flush_frame(out);
            }

            _ => {}
        }
    }

    /// Apply one page-selection update: repaint the union of the old / new
    /// highlight bounds (the paint phase has no selection knowledge) and
    /// overlay the new highlight, then refresh the copyable text.
    ///
    /// Backs `RenderCmd::TextSelection`, which is *incremental* rather than a
    /// complete snapshot: the repaint region is derived from the previous
    /// selection, so the command must never be superseded by a same-batch
    /// `RenderArena` / `Hover` — the batch merge defers it and runs it after
    /// the snapshot command instead (see the render thread's coalescing pass).
    fn apply_text_selection_cmd<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        anchor: Option<(u32, u32)>,
        end: Option<(u32, u32)>,
        mode: SelectionMode,
    ) {
        let new_sel = match (anchor, end) {
            (Some(a), Some(b)) => Some(TextSel {
                mode,
                anchor: a,
                end: b,
            }),
            _ => None,
        };
        let prev = self.text_selection;
        self.text_selection = new_sel;
        let Some(tl) = self.term_layout.as_mut() else {
            return;
        };
        let Some(geo) = self.geometry.as_ref() else {
            return;
        };
        let cols = tl.columns as u32;
        // Highlight bands are the actual cells the selection covers: flow
        // selections clip to the text columns, block selections keep the
        // rectangle.
        let prev_lines = match &prev {
            Some(n) => highlight_lines(&geo.arena, &geo.boxes, n, cols),
            None => Vec::new(),
        };
        let new_lines = match &new_sel {
            Some(n) => highlight_lines(&geo.arena, &geo.boxes, n, cols),
            None => Vec::new(),
        };
        // Repaint the union of the old and new selection bounds: the old
        // highlight must be cleared, the new one painted.
        let region = match (lines_bounds(&prev_lines), lines_bounds(&new_lines)) {
            (Some(a), Some(b)) => Some(union_rect(a, b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        if let Some(r) = region {
            tl.repaint_region(&geo.arena, &geo.boxes, r);
            tl.apply_text_selection(&new_lines, SELECTION_BG);
        }
        tl.render(out);
        flush_frame(out);
        // Refresh the copyable text of the selection (or clear it). Flow
        // selections are rebuilt from the layout's inline runs so wrapped
        // lines inside one block stay continuous (browser behaviour); block
        // selections keep the canvas extraction.
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
        if let Ok(mut guard) = self.selection_text.lock() {
            *guard = text;
        }
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
            // A resize invalidates the canvas-coordinate caches the overlay /
            // selection repaints rely on: drop them so the fresh frame paints
            // clean instead of re-applying stale rectangles onto the new
            // layout (the input loop separately clears its own mirrors).
            self.text_selection = None;
            self.open_select = None;
            if let Ok(mut guard) = self.selection_text.lock() {
                guard.clear();
            }
        }
        let Some(tl) = self.term_layout.as_mut() else {
            // `term_layout` is guaranteed `Some` by the branch above.
            return;
        };

        let boxes = tl.layout_only(&arena, root_id, term_map);

        // Install the document `<title>` as the terminal window title once
        // per session. The DOM is re-parsed on every `Redraw` but the title
        // is static, so this only runs on the first successful layout.
        if !self.title_set {
            if let Some(title) = extract_title(&arena, root_id) {
                execute!(stdout(), crossterm::terminal::SetTitle(title))
                    .unwrap_or_else(|e| log::error!("Failed to set terminal title: {}", e));
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
        let mut region = if !size_changed
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

        // The selection re-apply below blends each selected cell from its
        // freshly painted colour. Cells outside an *incremental* repaint
        // region would keep the previous frame's highlight and be blended
        // again — a step darker per repaint, which an animated widget (a
        // timer / hover repaint) turns into a visible bright / dark flicker.
        // Extend the incremental region to the selection bounds so every
        // selected cell is painted fresh exactly once before the highlight is
        // overlaid. A `None` region stays `None`: the full repaint already
        // covers the selection.
        if let Some(r) = region
            && let Some(sel) = &self.text_selection
        {
            let sel_lines = highlight_lines(&geo.arena, &geo.boxes, sel, tl.columns as u32);
            if let Some(sel_rect) = lines_bounds(&sel_lines) {
                region = Some(union_rect(r, sel_rect));
            }
        }

        match region {
            Some(r) => tl.repaint_region(&geo.arena, &geo.boxes, r),
            None => tl.repaint_from(&geo.arena, &geo.boxes),
        }

        // Clamp the requested scroll position against the *fresh* content
        // height — the paint pass just established `rendered_rows`, so this
        // keeps the user's position across a resize instead of jumping to the
        // top (a brand-new canvas still reports 0 rows before the paint) — and
        // publish the new upper bound of `scroll_y` for the input loop. The
        // bound must also come from the post-paint state: publishing 0 on a
        // fresh canvas (first frame / every resize) would clamp every scroll
        // position to 0, making the wheel and PageUp/PageDown appear dead
        // until some later re-layout republished it. u32::MAX is the startup
        // placeholder (no upper bound known yet), overwritten by the first
        // successful layout with the real value.
        tl.scroll_y = scroll_y;
        tl.clamp_scroll();
        self.max_scroll.store(tl.max_scroll(), Ordering::Relaxed);

        // Re-apply the active page selection: the paint pass has no knowledge
        // of it, so any freshly painted canvas needs the highlight overlaid.
        // Flow selections are clipped to the text columns so the highlight
        // never spills over borders / padding / trailing whitespace.
        if let Some(sel) = &self.text_selection {
            let lines = highlight_lines(&geo.arena, &geo.boxes, sel, tl.columns as u32);
            tl.apply_text_selection(&lines, SELECTION_BG);
        }
        // Re-apply an open dropdown overlay (a repaint would clear it).
        if let Some(open) = &self.open_select {
            apply_select_overlay(tl, open);
        }

        // The incremental renderer emits only the changed cells (and a
        // one-time full clear on the first frame / after resize), so redraws
        // need no per-frame Clear. `flush_frame` is the single I/O per frame,
        // skipped entirely when nothing changed.
        tl.render(out);
        flush_frame(out);
    }

    /// Re-cascade CSS (with the current `:hover` / `:focus`) into a
    /// script-engine arena snapshot and render it without re-parsing HTML.
    ///
    /// Backs `RenderCmd::RenderArena`: JS mutated the live DOM, so the arena
    /// arrives raw (no `css_attr`) and must be cascaded first. `dirty` lists
    /// the mutated nodes for the incremental repaint; `state_changed` is true
    /// when the `:hover` target or the focused field moved, which invalidates
    /// the incremental path (the highlight may live anywhere in the tree).
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
        state_changed: bool,
    ) {
        // `innerHTML`-injected `<style>` CSS must extend the session
        // stylesheet before the cascade. The added rules may restyle any node
        // in the tree, so the incremental repaint is disabled for this frame.
        let css_changed = !dirty.style_css.is_empty();
        if css_changed {
            self.append_style_css(&dirty.style_css);
        }
        let mut arena = arena.clone();
        reset_css_attr(&mut arena, root_id);
        css_into_node_with_state(&mut arena, root_id, &self.css_rules, &self.dynamic);
        let dirty = if state_changed || css_changed {
            None
        } else {
            Some(&dirty)
        };
        self.layout_and_paint(out, geo_tx, term_map, arena, root_id, scroll_y, zoom, dirty);
    }

    /// Extend the session stylesheet with `innerHTML`-injected CSS and
    /// re-parse the combined rules (new rules may add `:hover` / `:focus`
    /// selectors, so the pseudo-class rule set is recomputed).
    fn append_style_css(&mut self, css: &str) {
        if css.trim().is_empty() {
            return;
        }
        self.extra_css.push_str(css);
        self.extra_css.push('\n');
        let mut combined = String::with_capacity(self.base_css.len() + self.extra_css.len());
        combined.push_str(&self.base_css);
        combined.push('\n');
        combined.push_str(&self.extra_css);
        let rules = parse_css_source(&combined);
        self.pseudo_rules = css_pseudo_rule_set(&rules);
        self.css_rules = Arc::new(rules);
    }

    /// Re-cascade the cached geometry with the session interaction state
    /// (`self.dynamic`) and repaint only the region affected by `pseudo`
    /// moving from `prev` to `new` (the old highlight is cleared, the new one
    /// painted). When neither position matches a rule of that pseudo-class the
    /// cascade paints identically, so nothing is drawn.
    ///
    /// Backs `RenderCmd::Hover` / `RenderCmd::Focus`. The arena is cloned —
    /// the shared snapshot stays immutable for the hit-test thread, and a
    /// slotmap clone keeps the same keys, so the cached boxes remain valid
    /// (state changes only recolour, never reflow).
    fn repaint_dynamic<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        pseudo: PseudoClass,
        prev: Option<NodeId>,
        new: Option<NodeId>,
    ) {
        let Some(tl) = self.term_layout.as_mut() else {
            return;
        };
        let Some(geo) = self.geometry.as_ref() else {
            return;
        };
        let mut arena = geo.arena.clone();
        reset_css_attr(&mut arena, geo.root_id);
        css_into_node_with_state(&mut arena, geo.root_id, &self.css_rules, &self.dynamic);
        // Incremental repaint: only the union of the old and new affected
        // regions can differ from the previous frame. Repainting this small
        // region instead of resetting the whole canvas keeps hover / focus
        // tracking responsive.
        let region = match (
            pseudo_affected_region(&geo.boxes, &arena, &self.css_rules, pseudo, prev),
            pseudo_affected_region(&geo.boxes, &arena, &self.css_rules, pseudo, new),
        ) {
            (Some(a), Some(b)) => Some(union_rect(a, b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let Some(mut r) = region else {
            return;
        };
        // Extend the repaint to the active selection bounds: the selection
        // re-apply below blends each selected cell from its freshly painted
        // colour, so cells outside `r` would keep the previous frame's
        // highlight and be blended again (a step darker per hover / focus
        // repaint — the bright / dark flicker).
        if let Some(sel) = &self.text_selection {
            let sel_lines = highlight_lines(&geo.arena, &geo.boxes, sel, tl.columns as u32);
            if let Some(sel_rect) = lines_bounds(&sel_lines) {
                r = union_rect(r, sel_rect);
            }
        }
        tl.repaint_region(&arena, &geo.boxes, r);
        // The repaint reset the canvas (or part of it); the active selection
        // highlight must be overlaid again.
        if let Some(sel) = &self.text_selection {
            let lines = highlight_lines(&geo.arena, &geo.boxes, sel, tl.columns as u32);
            tl.apply_text_selection(&lines, SELECTION_BG);
        }
        if let Some(open) = &self.open_select {
            apply_select_overlay(tl, open);
        }
        tl.render(out);
        flush_frame(out);
    }

    /// Replace the session document and fully re-render it, keeping the
    /// current scroll position / zoom.
    ///
    /// Backs `RenderCmd::RenderHtml`. Script-side replacements are intercepted
    /// by `ScriptEngine::reload` (the engine stays the document's single
    /// source of truth, see `docs/browser-compat.md` §6.4.12); this
    /// render-thread path remains only for direct API use and as a fallback
    /// when an in-engine reload fails.
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
        // A fresh document discards any `innerHTML`-injected CSS.
        self.extra_css = String::new();
        let rules = parse_css_source(&self.base_css);
        self.pseudo_rules = css_pseudo_rule_set(&rules);
        self.css_rules = Arc::new(rules);
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
    let bottom =
        a.y.saturating_add(a.height)
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

/// Paint an open `<select>` dropdown overlay onto the canvas: clear every
/// row of the overlay rectangle, then write each option label with the
/// highlighted row on the selection background and the rest on the overlay
/// background. Overlays cover whatever the layout painted underneath.
fn apply_select_overlay(tl: &mut TermLayout, open: &OpenSelect) {
    let cols = tl.columns as usize;
    let max = tl.layout.len();
    // The overlay may extend below the layout's content (a `<select>` near
    // the bottom of a short document): grow the canvas buffer so every
    // option row is paintable. This does not touch `rendered_rows`, so the
    // overlay never affects the scroll range.
    let needed = open
        .rect
        .y
        .saturating_add(open.rect.height)
        .saturating_mul(cols);
    if needed > max {
        tl.layout.resize(needed, Block::default());
    }
    let max = tl.layout.len();
    let x_end = open.rect.x.saturating_add(open.rect.width).min(cols);
    for (row_idx, row) in open.rows.iter().enumerate() {
        let row_start = open.rect.y.saturating_add(row_idx).saturating_mul(cols);
        if row_start >= max {
            break;
        }
        let (label, fg, is_item) = match row {
            // `<optgroup>` headers are dimmed and never take the highlight.
            SelectRow::Group(g) => (g.as_str(), GROUP_FG, false),
            SelectRow::Item(i) => (
                open.options.get(*i).map(|o| o.label.as_str()).unwrap_or(""),
                OVERLAY_FG,
                true,
            ),
        };
        let bg = if is_item && row_idx == open.highlight_row() {
            Some(SELECTION_BG)
        } else {
            Some(OVERLAY_BG)
        };
        // Clear the whole row, then write the label.
        for x in open.rect.x..x_end {
            let idx = row_start.saturating_add(x);
            if idx >= max {
                break;
            }
            let cell = &mut tl.layout[idx];
            cell.word.clear();
            cell.word.push(' ');
            cell.fcolor = Some(fg);
            cell.bcolor = bg;
            cell.style = FontStyle::empty();
        }
        let mut col = open.rect.x;
        for ch in label.chars() {
            let w = char_width(ch).max(1);
            if col >= x_end {
                break;
            }
            let idx = row_start.saturating_add(col);
            if idx < max {
                let cell = &mut tl.layout[idx];
                cell.word.clear();
                cell.word.push(ch);
                cell.fcolor = Some(fg);
                cell.bcolor = bg;
                cell.style = FontStyle::empty();
            }
            col = col.saturating_add(w);
        }
    }
}
fn highlight_lines(
    arena: &NodeArena,
    boxes: &[LayoutBox],
    sel: &TextSel,
    cols: u32,
) -> Vec<(u32, u32, u32)> {
    let raw = selection_lines(sel, cols);
    match sel.mode {
        SelectionMode::Flow => crate::layout::selection_text_lines(arena, boxes, &raw),
        SelectionMode::Block => raw,
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

/// Union of every box rect that a dynamic pseudo-class's state target being
/// `target` can change: the target plus (for `:hover`) each ancestor that
/// matches a rule of that pseudo-class. Candidates that match no rule are
/// skipped — otherwise hovering a bottom-of-page button would repaint the
/// whole document through its `body` ancestor.
///
/// Used by `repaint_dynamic` (backing the `Hover` / `Focus` commands) to
/// repaint only the affected region instead of the whole canvas.
fn pseudo_affected_region(
    boxes: &[LayoutBox],
    arena: &NodeArena,
    rules: &CssRule,
    pseudo: PseudoClass,
    target: Option<NodeId>,
) -> Option<Rect> {
    let match_state = pseudo.state_with_target(target);
    let mut region: Option<Rect> = None;
    for id in pseudo.affected_candidates(arena, target) {
        // Only a selector that contains `pseudo` and matches `id` while the
        // state target is `target` can repaint `id`'s boxes.
        let affected = rules.0.iter().any(|entry| {
            let sel = &entry.selector;
            PseudoClassSet::from_selector(sel).contains_pseudo(pseudo)
                && selector_chain_matches_with_state(arena, id, sel, &match_state)
        });
        if affected {
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
        }
    }
    region
}

fn render_thread(
    rx: mpsc::Receiver<RenderCmd>,
    html: Arc<str>,
    base_css: Arc<str>,
    max_scroll: Arc<AtomicU32>,
    geo_tx: Option<mpsc::Sender<Arc<Geometry>>>,
    selection_text: Arc<Mutex<String>>,
    render_beat: Arc<AtomicU64>,
) {
    // One persistent write buffer for the whole session: every frame writes
    // into it and is flushed exactly once, so a frame costs a single write
    // syscall instead of one per render call.
    let mut out = BufWriter::new(stdout());
    let css_rules = Arc::new(parse_css_source(&base_css));
    let pseudo_rules = css_pseudo_rule_set(&css_rules);
    let mut state = RenderState {
        html,
        base_css,
        extra_css: String::new(),
        css_rules,
        pseudo_rules,
        term_layout: None,
        title_set: false,
        geometry: None,
        dynamic: DynamicState::default(),
        text_selection: None,
        selection_text,
        open_select: None,
        max_scroll,
    };

    let mut last_frame_at: Option<Instant> = None;
    loop {
        let first = match rx.recv() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Render channel disconnected: {}", e);
                break;
            }
        };
        // Rate-limit frames: when commands arrive faster than the console can
        // drain a frame (a fast wheel scroll can push a full-viewport diff at
        // every tick), the render thread would otherwise queue frame after
        // frame, saturate the output pipe and stall on the write — which
        // looks exactly like the whole app froze. Waiting out the remainder of
        // `MIN_FRAME_GAP` lets the `try_recv` drain below coalesce the burst
        // into one frame and bounds the ANSI output rate.
        if let Some(t) = last_frame_at {
            let since = t.elapsed();
            if since < MIN_FRAME_GAP {
                std::thread::sleep(MIN_FRAME_GAP - since);
            }
        }
        // Commands split into two classes with different merge semantics:
        //
        // - Stateful, must-run-in-order commands — `RenderInput` (a focus
        //   switch queues blur + focus for two fields; dropping either leaves
        //   a stale caret) and `Select*` (they mutate the render thread's
        //   dropdown state, so a `SelectClose` must not be dropped in favour
        //   of the `RenderArena` its `change` handler triggers). They are
        //   collected in arrival order and all run.
        // - Snapshot commands (`Rerender` / `Redraw` / `RenderArena` /
        //   `RenderHtml` / `Hover` / `Quit`) — each carries a complete final
        //   state, so a later one safely supersedes an earlier one; only the
        //   last of the batch is kept.
        // - `TextSelection` is *incremental*: its repaint region derives from
        //   the previously applied selection, so superseding it with a
        //   same-batch snapshot (a `RenderArena` triggered by a timer / hover
        //   mid-drag) would leave the render thread re-applying the stale
        //   highlight and make the next selection jump the edge forward — the
        //   visible flicker. It is deferred instead and applied after the
        //   snapshot command, on the fresh canvas.
        // - A `Hover` arriving after a pending `RenderArena` is likewise
        //   deferred: superseding the arena render (whose `dynamic.hovered`
        //   was captured when the DOM change was folded) would drop the DOM
        //   mutation — an animated widget under the pointer (e.g. the music
        //   demo's progress bar) would then update in irregular jumps. The
        //   deferred hover repaint re-applies the newer highlight on top.
        let mut pending_inputs: Vec<(NodeId, String, usize, bool, Option<(usize, usize)>)> =
            Vec::new();
        let mut pending_selects: Vec<RenderCmd> = Vec::new();
        let mut pending_text_sel: Option<RenderCmd> = None;
        let mut pending_hover: Option<RenderCmd> = None;
        let mut cmd = match first {
            RenderCmd::RenderInput {
                node_id,
                value,
                cursor,
                focused,
                selection,
            } => {
                pending_inputs.push((node_id, value, cursor, focused, selection));
                None
            }
            sel @ (RenderCmd::SelectOpen { .. }
            | RenderCmd::SelectClose { .. }
            | RenderCmd::SelectMove { .. }) => {
                pending_selects.push(sel);
                None
            }
            other @ RenderCmd::TextSelection { .. } => {
                pending_text_sel = Some(other);
                None
            }
            other => Some(other),
        };
        while let Ok(next) = rx.try_recv() {
            match next {
                RenderCmd::RenderInput {
                    node_id,
                    value,
                    cursor,
                    focused,
                    selection,
                } => pending_inputs.push((node_id, value, cursor, focused, selection)),
                sel @ (RenderCmd::SelectOpen { .. }
                | RenderCmd::SelectClose { .. }
                | RenderCmd::SelectMove { .. }) => pending_selects.push(sel),
                other @ RenderCmd::TextSelection { .. } => {
                    // Only the latest selection matters; earlier ones in the
                    // same batch are intermediates.
                    pending_text_sel = Some(other);
                }
                other @ RenderCmd::Hover { .. }
                    if matches!(cmd, Some(RenderCmd::RenderArena { .. })) =>
                {
                    // The arena render re-cascades with the hover state it
                    // carried; the newer hover repaint must not swallow the
                    // DOM mutations, so defer it (see the merge comment).
                    pending_hover = Some(other);
                }
                other => {
                    cmd = Some(other);
                    // Quit supersedes anything still queued in the batch.
                    if matches!(cmd, Some(RenderCmd::Quit)) {
                        break;
                    }
                }
            }
        }
        for (node_id, value, cursor, focused, selection) in pending_inputs {
            state.handle_input(&mut out, node_id, value, cursor, focused, selection);
        }
        for sel_cmd in pending_selects {
            state.handle_select(&mut out, sel_cmd);
        }

        // Publish the frame-completion heartbeat *before* processing: the
        // input loop's watchdog uses it to tell a *stuck* render thread
        // (infinite layout loop, an enormous canvas allocation, a blocked
        // console write) apart from an idle or no-op one, which is still
        // alive and must not trip the watchdog.
        render_beat.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            Ordering::Relaxed,
        );
        last_frame_at = Some(Instant::now());

        // Process the deferred `TextSelection` *before* the snapshot command:
        // its repaint uses the current canvas (the union of the old and new
        // bounds), and the snapshot re-applies it on the fresh canvas — so it
        // is never lost to a snapshot arm's early `continue` (see the merge
        // comment above).
        if let Some(RenderCmd::TextSelection { anchor, end, mode }) = pending_text_sel {
            state.apply_text_selection_cmd(&mut out, anchor, end, mode);
        }

        if let Some(cmd) = cmd {
            let batch_t0 = Instant::now();
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
                    dynamic,
                    dirty,
                } => {
                    // A hover / focus change re-styles the whole tree (the
                    // `:hover` / `:focus` highlight may sit anywhere), so the
                    // incremental repaint is only valid when the interaction
                    // state is unchanged.
                    let state_changed = state.dynamic != dynamic;
                    state.dynamic = dynamic;
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
                        state_changed,
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
                    let prev = state.dynamic.hovered;
                    state.dynamic.hovered = node_id;
                    // Without any `:hover` rule the cascade is identical for every
                    // pointer position, so nothing can change on screen — skip the
                    // repaint entirely (browser behaviour: moving the pointer with
                    // no `:hover` styles produces no drawing).
                    if !state.pseudo_rules.contains_pseudo(PseudoClass::Hover) {
                        continue;
                    }
                    state.repaint_dynamic(&mut out, PseudoClass::Hover, prev, node_id);
                }

                RenderCmd::Focus { node_id } => {
                    let prev = state.dynamic.focused;
                    state.dynamic.focused = node_id;
                    // Without any `:focus` rule the cascade is identical for every
                    // focus position, so nothing can change on screen — skip the
                    // repaint entirely (browser behaviour: focusing with no
                    // `:focus` styles produces no drawing).
                    if !state.pseudo_rules.contains_pseudo(PseudoClass::Focus) {
                        continue;
                    }
                    state.repaint_dynamic(&mut out, PseudoClass::Focus, prev, node_id);
                }

                // Select dropdown commands share one handler so the pending pass
                // can also run them (see `pending_selects` above).
                RenderCmd::SelectOpen { .. }
                | RenderCmd::SelectClose { .. }
                | RenderCmd::SelectMove { .. } => {
                    state.handle_select(&mut out, cmd);
                }

                RenderCmd::TextSelection { anchor, end, mode } => {
                    state.apply_text_selection_cmd(&mut out, anchor, end, mode);
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

                // JS `el.focus()` / `el.blur()` requests are intercepted by the
                // input loop (they mutate input-loop state); if one ever reaches
                // the render thread it is stale — drop it.
                RenderCmd::FocusRequest { .. } | RenderCmd::BlurRequest { .. } => {
                    log::debug!("Focus request reached the render thread — dropping");
                }
            }

            // A single command batch taking this long usually means a bogus
            // canvas size or a blocked console write — log it so a "freeze" can
            // be traced to its exact cause instead of staying mysterious.
            let elapsed = batch_t0.elapsed();
            if elapsed > Duration::from_millis(200) {
                log::warn!("渲染线程单批命令处理耗时 {elapsed:?}");
            }
        }

        // A deferred `Hover` runs last so its repaint re-applies the newer
        // hover highlight on top of both the arena render and the selection.
        if let Some(RenderCmd::Hover { node_id }) = pending_hover {
            let prev = state.dynamic.hovered;
            state.dynamic.hovered = node_id;
            if state.pseudo_rules.contains_pseudo(PseudoClass::Hover) {
                state.repaint_dynamic(&mut out, PseudoClass::Hover, prev, node_id);
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

/// Fold one `ScriptResponse::Event` into the current frame: hover updates,
/// host-pushed render commands, and — when JS mutated the DOM — a fresh arena
/// snapshot + dirty set forwarded as a (region-repaint) `RenderArena`.
/// Shared by the key-gate wait and the ordinary async reply fold.
fn fold_script_event(
    state: &InputState,
    cmds: &mut Vec<RenderCmd>,
    hovered: &mut Option<NodeId>,
    live_dom: &mut Option<(Arc<NodeArena>, NodeId)>,
    ev: crate::script::ScriptEvent,
) {
    *hovered = ev.hovered;
    cmds.extend(ev.cmds);
    if let Some(d) = ev.dirty {
        *live_dom = Some((Arc::clone(&d.arena), d.root_id));
        cmds.push(RenderCmd::RenderArena {
            arena: d.arena,
            root_id: d.root_id,
            scroll_y: state.scroll_y,
            zoom: state.zoom,
            dynamic: DynamicState {
                hovered: ev.hovered,
                focused: state.focused_input,
            },
            dirty: d.dirty,
        });
    }
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
    if let Err(e) = execute!(
        stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        Hide
    ) {
        log::error!("Failed to enter alternate screen: {}", e);
        if let Err(e2) = disable_raw_mode() {
            log::error!("Failed to disable raw mode during cleanup: {}", e2);
        }
        return;
    }

    let css_source = Arc::<str>::from(combined_css_source(html, css));

    let html: Arc<str> = Arc::from(html);

    // Shared scroll upper bound — initial value `u32::MAX` means "not yet
    // known" (the first layout hasn't run); the render thread overwrites it
    // on every successful render with the real `tl.max_scroll()`.
    let max_scroll = Arc::new(AtomicU32::new(u32::MAX));

    // Geometry snapshot channel (render thread → hit-test thread) and the
    // published hit map (hit-test thread → input loop + script thread). The
    // hit-test thread is only spawned when event handling is requested.
    let (geo_tx, geo_rx) = mpsc::channel::<Arc<Geometry>>();
    let hit_map: Arc<Mutex<Arc<HitMap>>> = Arc::new(Mutex::new(Arc::new(HitMap::new())));

    // The script engine owns the live DOM when events are enabled; it parses
    // the document itself (arena + `<script>` blocks) and reports dirty flags
    // when JS mutates the DOM. It runs on a dedicated script thread so a
    // blocking host function never stalls input handling (see
    // `script::msg::run_script_thread`).
    let (script_tx, script_rx) = mpsc::channel::<ScriptMsg>();
    let (script_out_tx, script_out_rx) = mpsc::channel::<ScriptResponse>();
    let script_thread: Option<std::thread::JoinHandle<()>> = bindings.map(|bindings| {
        let html = Arc::clone(&html);
        let hit_map_thread = Arc::clone(&hit_map);
        std::thread::spawn(move || {
            run_script_thread(html, bindings, script_rx, script_out_tx, hit_map_thread)
        })
    });

    // Wait for the engine to come up — the first frame needs its live arena.
    // `live_dom` is the input loop's latest DOM snapshot for read-only queries;
    // it is refreshed whenever JS mutates the arena.
    let mut live_dom: Option<(Arc<NodeArena>, NodeId)> = None;
    if script_thread.is_some() {
        match script_out_rx.recv() {
            Ok(ScriptResponse::Ready { arena, root_id }) => live_dom = Some((arena, root_id)),
            Ok(ScriptResponse::Failed(err)) => {
                log::error!("Failed to initialize script engine: {err}");
            }
            Ok(ScriptResponse::Event(_)) => {
                log::error!("Script thread sent an event before Ready");
            }
            Err(e) => log::error!("Script thread init channel error: {e}"),
        }
    }
    let events_active = live_dom.is_some();
    if events_active {
        let hit_map_thread = Arc::clone(&hit_map);
        std::thread::spawn(move || hit_test_thread(geo_rx, hit_map_thread));
    }

    // Text of the current page selection, refreshed by the render thread and
    // read by the input loop on `Ctrl+C` (shared through `Arc<Mutex<_>>`).
    let selection_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    let (tx, rx) = mpsc::channel::<RenderCmd>();
    // Frame-completion heartbeat: the render thread bumps it after every
    // processed command batch; the input loop's watchdog below uses it to
    // distinguish a *stuck* render thread from an idle one.
    let render_beat: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let render_handle = std::thread::spawn({
        let html_clone = Arc::clone(&html);
        let css_clone = Arc::clone(&css_source);
        let max_scroll_clone = Arc::clone(&max_scroll);
        let selection_text_clone = Arc::clone(&selection_text);
        let render_beat_clone = Arc::clone(&render_beat);
        let geo_tx = events_active.then_some(geo_tx);
        move || {
            render_thread(
                rx,
                html_clone,
                css_clone,
                max_scroll_clone,
                geo_tx,
                selection_text_clone,
                render_beat_clone,
            );
        }
    });

    let mut state = InputState::new();
    state.selection_text = Arc::clone(&selection_text);
    let (mut columns, mut rows): (u16, u16) = match TermMap::get_ter_map() {
        Ok(m) => (m.columns, m.rows),
        Err(e) => {
            log::error!("Failed to get terminal size: {}", e);
            (80, 24)
        }
    };

    // First frame: the engine's live arena (scripts already ran and may have
    // mutated the DOM) or a plain parse-based redraw.
    let first_cmd = match &live_dom {
        Some((arena, root_id)) => RenderCmd::RenderArena {
            arena: Arc::clone(arena),
            root_id: *root_id,
            scroll_y: state.scroll_y,
            zoom: state.zoom,
            dynamic: DynamicState::default(),
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

    // Current `:hover` target, reported by the script thread after every
    // pointer message; feeds `RenderArena.hovered` so re-cascades re-apply
    // the highlight.
    let mut hovered: Option<NodeId> = None;

    let mut quit = false;
    // Render-stall watchdog state: `(heartbeat at last command send, when it
    // was sent)`. The render thread bumps its heartbeat after each processed
    // batch, so a send followed by no heartbeat for a while means the render
    // thread is stuck (not merely idle — an idle render thread receives no
    // commands, so no send happens and this stays `None`).
    let mut render_stall: Option<(u64, Instant)> = None;
    while !quit {
        if let Some((beat_at_send, sent_at)) = render_stall {
            let beat = render_beat.load(Ordering::Relaxed);
            if beat != beat_at_send {
                // A frame completed — the render thread is alive.
                render_stall = None;
            } else if sent_at.elapsed() > Duration::from_secs(2) {
                log::warn!(
                    "渲染线程疑似卡死：{} 秒未完成任何帧（心跳停在 {})",
                    sent_at.elapsed().as_secs(),
                    beat_at_send
                );
                // Throttle: do not re-warn every iteration while stuck.
                render_stall = None;
            }
        }

        // Detect a dead render thread (e.g. from a panic) promptly. Rust's
        // spawned-thread panic handler swallows the panic and kills just the
        // worker, so without this check the main loop would keep polling
        // events, fail to send, and only exit when the user presses a key.
        if render_handle.is_finished() {
            log::error!("Render thread has terminated unexpectedly");
            break;
        }

        // A dead script thread must not be silently ignored either: without
        // this check the loop would keep sending messages into a void and
        // hover / events would freeze while scrolling still worked.
        if events_active && script_thread.as_ref().is_some_and(|h| h.is_finished()) {
            log::error!("Script thread has terminated unexpectedly");
            break;
        }

        let mut msgs: Vec<ScriptMsg> = Vec::new();
        // Detect terminal size changes ourselves instead of relying solely on
        // `Event::Resize`: some resize paths never emit the event (a legacy
        // Windows console can resize its window without changing the buffer),
        // and after such a resize the render thread keeps drawing to a stale
        // viewport while page-step keys use a stale `rows` — together this
        // looks like input is frozen. Polling per iteration re-syncs both
        // within one loop pass and (on Windows) re-applies the console input
        // mode the resize may have reset. The size is normalized exactly like
        // `TermMap::get_ter_map` so both sides always agree.
        let mut size_changed = false;
        match size() {
            Ok((c, r)) => {
                // Clamp the same way `TermMap::get_ter_map` does: a transiently
                // inverted Windows `SMALL_RECT` during a resize can report
                // values like 65535 (i16 → u16 wrap), which would make the
                // render thread allocate a gigantic canvas and freeze the app.
                let c = c.clamp(1, MAX_TERM_COLS);
                let r = r.clamp(1, MAX_TERM_ROWS);
                size_changed = c != columns || r != rows;
                if size_changed {
                    log::info!("终端尺寸变化: {}x{} -> {}x{}", columns, rows, c, r);
                }
                columns = c;
                rows = r;
            }
            Err(e) => log::error!("Failed to get terminal size: {}", e),
        }
        // Re-apply the console input mode every iteration, not just when the
        // size *changed*: a Windows resize can reset it asynchronously (the
        // console finishes the resize after our re-apply), and once the size
        // stabilises nothing else would ever re-enable mouse capture / raw
        // mode — mouse and keyboard events stay dead until the next resize.
        // The calls are cheap and idempotent (no-op off Windows).
        reapply_console_modes();

        // Mirror the render thread's scroll clamp before computing any input:
        // a terminal grow (or a document reload) can shrink the content below
        // the current `scroll_y`. Without this the boundary checks below
        // disagree with the on-screen position — every wheel / arrow `Rerender`
        // gets clamped to the same value in the render thread and dropped as a
        // no-op, so scrolling appears dead after a resize.
        let max_scroll_val = max_scroll.load(Ordering::Relaxed);
        state.clamp_scroll_to(max_scroll_val);
        let mut cmds = if events_active {
            let map = match hit_map.lock() {
                Ok(guard) => Arc::clone(&guard),
                Err(e) => {
                    log::error!("Hit map lock poisoned: {}", e);
                    break;
                }
            };
            // A pending viewport change has now reached the *rebuilt* hit map
            // (the resize render finished and the hit-test thread published
            // it), so re-validate the pointer hover against the fresh
            // geometry: the element under a stationary pointer may have
            // reflowed to something else, and a synthetic `Move` fires the
            // correct MouseOut / MouseOver (+ a fresh `Hover` highlight).
            if state.take_resize_revalidation()
                && let Some((col, row)) = state.last_mouse
            {
                let (x, y) = (col as u32, row as u32 + state.scroll_y);
                msgs.push(ScriptMsg::Move { x, y });
            }
            trigger_refresh(
                &mut state,
                rows,
                max_scroll_val,
                Some(&map),
                live_dom.as_ref().map(|(arena, _)| arena.as_ref()),
                &mut msgs,
                Some(Duration::from_millis(100)),
            )
        } else {
            trigger_refresh(
                &mut state,
                rows,
                max_scroll_val,
                None,
                None,
                &mut msgs,
                Some(Duration::from_millis(100)),
            )
        };

        // A resize invalidates every canvas coordinate the input loop caches:
        // without this a press whose release the console dropped during the
        // resize would stick and turn all later motion into a drag (hover /
        // click events go silent), and a stale text selection / open dropdown
        // overlay would paint wrong cells.
        if size_changed {
            state.invalidate_viewport(&mut cmds);
        }

        // Force a re-layout against the new viewport. When the script engine
        // owns the document this is converted into a `RenderArena` below (so
        // JS DOM mutations survive the re-layout); the render thread merges it
        // with any event-driven `Redraw` from the same batch.
        if size_changed {
            cmds.push(RenderCmd::Redraw {
                scroll_y: state.scroll_y,
                zoom: state.zoom,
            });
            // The pointer hover must be re-validated once the re-layout's
            // fresh hit map is published (consumed next iteration, above).
            state.request_resize_revalidation();
        }

        // Hand the actions that need the engine to the script thread (the
        // channel is unbounded, so a busy script thread never blocks us).
        for msg in msgs {
            if let Err(e) = script_tx.send(msg) {
                log::error!("Failed to send script message: {}", e);
                quit = true;
                break;
            }
        }

        // Gate a page-level built-in key (scroll / zoom / quit): the script
        // thread's `keydown` response reports whether JS called
        // `preventDefault()`. Clear verdict → the deferred commands are
        // dropped (the page owns that key); no verdict within
        // `KEY_GATE_TIMEOUT` → run the built-in anyway, so a busy or blocked
        // script thread never stalls scrolling / quitting (decoupling). The
        // wait also folds any responses that arrive meanwhile (hover, DOM
        // mutations from key handlers).
        if let Some(gate) = state.key_gate.take() {
            let mut gate_cmds = gate.cmds;
            let mut resolved = false;
            while !resolved {
                match script_out_rx.recv_timeout(KEY_GATE_TIMEOUT) {
                    Ok(ScriptResponse::Event(ev)) => {
                        let verdict = ev.key_gate.filter(|(id, _)| *id == gate.id);
                        fold_script_event(&state, &mut cmds, &mut hovered, &mut live_dom, ev);
                        if let Some((_, prevented)) = verdict {
                            if !prevented {
                                log::debug!(
                                    "Key gate cleared for key id {} — applying built-in",
                                    gate.id
                                );
                                cmds.extend(std::mem::take(&mut gate_cmds));
                            }
                            resolved = true;
                        }
                    }
                    Ok(ScriptResponse::Ready { .. } | ScriptResponse::Failed(_)) => {
                        log::error!("Unexpected one-shot script response during the key gate");
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        // Script thread busy / blocked in a host function:
                        // fall back to the built-in so input stays responsive.
                        log::debug!(
                            "Key gate timed out for key id {} — running built-in",
                            gate.id
                        );
                        cmds.extend(std::mem::take(&mut gate_cmds));
                        resolved = true;
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        log::error!("Script thread disconnected during the key gate");
                        cmds.extend(std::mem::take(&mut gate_cmds));
                        resolved = true;
                    }
                }
            }
        }

        // Fold the script thread's replies into this frame: render commands
        // pushed by host functions, hover updates, and — when JS mutated the
        // DOM — a fresh arena snapshot + dirty set for a (region) repaint.
        for resp in script_out_rx.try_iter() {
            match resp {
                ScriptResponse::Event(ev) => {
                    fold_script_event(&state, &mut cmds, &mut hovered, &mut live_dom, ev);
                }
                ScriptResponse::Ready { .. } | ScriptResponse::Failed(_) => {
                    log::error!("Unexpected one-shot script response in the input loop");
                }
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
                    RenderCmd::Focus { .. } if arena_pending => {
                        // Dropped: the arena render re-cascades with `focused`.
                    }
                    other => merged.push(other),
                }
            }
            merged
        };

        // JS `el.focus()` / `el.blur()` requests target input-loop state (the
        // edit session lives here) and route their `focus` / `blur` (+ `change`)
        // events back through the script-message path — apply them now and
        // never forward them to the render thread. `msgs` was already sent
        // above, so the focus transitions flush their own messages immediately.
        let cmds = {
            let mut focus_msgs: Vec<ScriptMsg> = Vec::new();
            let cmds = apply_focus_requests(
                &mut state,
                cmds,
                live_dom.as_ref().map(|(a, _)| a.as_ref()),
                &mut focus_msgs,
            );
            for msg in focus_msgs {
                if let Err(e) = script_tx.send(msg) {
                    log::error!("Failed to send focus message: {}", e);
                    quit = true;
                    break;
                }
            }
            cmds
        };

        let mut sent_any = false;
        for cmd in cmds {
            if matches!(cmd, RenderCmd::Quit) {
                if let Err(e) = tx.send(RenderCmd::Quit) {
                    log::error!("Failed to send Quit: {}", e);
                }
                quit = true;
                break;
            }
            sent_any = true;
            // Capture whether this was a resize / zoom `Redraw` *before* the
            // events-active rewrite below turns it into a `RenderArena` — the
            // terminal-height refresh must run in both paths or PageUp /
            // PageDown keep stepping with the pre-resize row count.
            let was_redraw = matches!(cmd, RenderCmd::Redraw { .. });
            // With the script engine active, the engine's live arena is the
            // source of truth: convert parse-based `Redraw` (zoom / resize)
            // into a `RenderArena` so JS DOM mutations survive re-renders.
            let cmd = match (cmd, events_active.then_some(())) {
                (RenderCmd::Redraw { scroll_y, zoom }, Some(())) => match &live_dom {
                    Some((arena, root_id)) => RenderCmd::RenderArena {
                        arena: Arc::clone(arena),
                        root_id: *root_id,
                        scroll_y,
                        zoom,
                        dynamic: DynamicState {
                            hovered,
                            focused: state.focused_input,
                        },
                        dirty: DirtyInfo::default(),
                    },
                    None => RenderCmd::Redraw { scroll_y, zoom },
                },
                (other, _) => other,
            };
            if was_redraw && let Ok(map) = TermMap::get_ter_map() {
                rows = map.rows;
            }
            if let Err(e) = tx.send(cmd) {
                log::error!("Failed to send render command: {}", e);
                quit = true;
                break;
            }
        }
        if !quit && sent_any {
            // Arm the render-stall watchdog: remember the heartbeat as of this
            // batch so the next loop iteration can tell "alive" from "stuck".
            render_stall = Some((render_beat.load(Ordering::Relaxed), Instant::now()));
        }
    }

    // Ask the script thread to stop. It is not joined — the process is about
    // to exit, and a host function blocked inside it must not hang shutdown
    // (it holds no terminal resources; it only talks through channels).
    if let Some(handle) = script_thread {
        if script_tx.send(ScriptMsg::Quit).is_err() {
            log::debug!("Script thread already gone; skipping Quit");
        }
        drop(handle);
    }

    if let Err(payload) = render_handle.join() {
        log::error!(
            "Render thread panicked: {}",
            panic_message(payload.as_ref())
        );
    }

    if let Err(e) = execute!(
        stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
        Show
    ) {
        log::error!("Failed to leave alternate screen: {}", e);
    }
    if let Err(e) = disable_raw_mode() {
        log::error!("Failed to disable raw mode: {}", e);
    }
}
#[cfg(debug_assertions)]
use std::backtrace::Backtrace;
#[cfg(debug_assertions)]
use std::panic;
#[cfg(debug_assertions)]
use std::sync::Once;

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

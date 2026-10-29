//! Render-thread per-session state (`RenderState`) and its interaction
//! handlers: input fields, `<select>` dropdowns and page text selection.

use std::io::{BufWriter, Write};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use compact_str::CompactString;
use crossterm::style::Color;

use crate::events::SharedGeometry;
use crate::html::css_parse::Viewport;
use crate::html::css_parse::{self, CssRule, DynamicState, PseudoClassSet};
use crate::html::html_parse::ParsedDocument;
use crate::layout::Rect;
use crate::layout::layoutbox::geometry::{
    SelectRow, overlay_options, select_item_indices, select_overlay_rect, select_rows,
};
use crate::source::SharedSourceText;
use crate::tdom::node::{NodeArena, NodeId};
use crate::tui::graph_api::{
    InputRenderState, RenderCmd, SelectionMode, SharedScrollbarVisibility, TermLayout,
};

use super::overlay::apply_select_overlay;
use super::pipeline::flush_frame;
use super::region::{find_box_rect, union_rect};
use super::selection::{highlight_lines, lines_bounds, selection_lines};

pub(super) fn cascade_document(
    document: &ParsedDocument,
    css_rules: &CssRule,
    viewport: Viewport,
) -> (NodeId, NodeArena) {
    let mut arena = (*document.arena).clone();
    css_parse::css_into_node_with_state(
        &mut arena,
        document.root_id,
        css_rules,
        &DynamicState {
            viewport,
            ..Default::default()
        },
    );
    (document.root_id, arena)
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
pub(super) const SELECTION_BG: Color = Color::Rgb {
    r: 0x3b,
    g: 0x42,
    b: 0x61,
};

/// Dropdown overlay background (Tokyo Night panel).
pub(super) const OVERLAY_BG: Color = Color::Rgb {
    r: 0x24,
    g: 0x28,
    b: 0x3b,
};

/// Overlay text colour (Tokyo Night foreground).
pub(super) const OVERLAY_FG: Color = Color::Rgb {
    r: 0xc0,
    g: 0xca,
    b: 0xf5,
};

/// Foreground of `<optgroup>` header rows — dimmer than the options so a
/// group label reads as a non-option heading (clicking it still picks the
/// group's first option).
pub(super) const GROUP_FG: Color = Color::Rgb {
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
pub(super) struct TextSel {
    pub(super) mode: SelectionMode,
    pub(super) anchor: (u32, u32),
    pub(super) end: (u32, u32),
}

/// An open `<select>` dropdown overlay: the field rect, the option list and
/// the highlighted row. Painted after every repaint (like the text
/// selection) so it survives layout / hover passes.
///
/// `rect` / `options` / `rows` are *derived* state: every repaint of the
/// overlay re-resolves them against the layout being painted
/// ([`Self::reanchor`]) so the dropdown always sits on the frame it is
/// drawn on — a re-layout that moves the field or changes the option list
/// must not leave the overlay floating at the position cached when it
/// opened. `node_id` + `filter` + `highlight` are the intent that survives.
pub(super) struct OpenSelect {
    pub(super) node_id: NodeId,
    /// The datalist filter the overlay was opened with (`None` for a plain
    /// `<select>`); re-applied when the options are re-resolved.
    pub(super) filter: Option<CompactString>,
    /// Overlay rectangle (canvas coordinates, directly below the field).
    /// Re-derived on every reanchor — do not paint from a stale value.
    pub(super) rect: Rect,
    /// Flat option list (`value`, `label`, optional group) — the label source
    /// for the row painter. Re-resolved on every reanchor.
    pub(super) options: Vec<crate::layout::layoutbox::geometry::SelectOption>,
    /// Dropdown rows (group headers interleaved with option items).
    /// Re-resolved on every reanchor.
    pub(super) rows: Vec<crate::layout::layoutbox::geometry::SelectRow>,
    /// Option index highlighted by the keyboard cursor.
    pub(super) highlight: usize,
}

impl OpenSelect {
    /// The row index holding the highlighted option (first row when the
    /// highlight is stale / absent).
    pub(super) fn highlight_row(&self) -> usize {
        self.rows
            .iter()
            .position(|r| matches!(r, SelectRow::Item(i) if *i == self.highlight))
            .unwrap_or(0)
    }

    /// Re-resolve the overlay's options / rows / rect against a fresh layout.
    ///
    /// A re-layout (`RenderArena` / `RenderDocument` / `Redraw` / a pinned-box
    /// scroll repaint) can move the field or change the option list; the
    /// overlay must be re-derived from the geometry it will be painted on.
    /// Returns `false` when the select no longer produces a box (the
    /// document was replaced or the field is hidden) — the caller must drop
    /// the overlay instead of painting it into nowhere.
    pub(super) fn reanchor(
        &mut self,
        arena: &NodeArena,
        boxes: &[crate::layout::LayoutBox],
    ) -> bool {
        let Some(select_rect) = find_box_rect(boxes, self.node_id) else {
            return false;
        };
        let options = overlay_options(arena, self.node_id, self.filter.as_deref());
        let rows = select_rows(&options);
        // Keep the keyboard highlight when the option list still holds it;
        // clamp when the list shrank below it.
        let item_count = select_item_indices(&rows).len();
        self.highlight = self.highlight.min(item_count.saturating_sub(1));
        self.rect = select_overlay_rect(&options, &rows, select_rect);
        self.options = options;
        self.rows = rows;
        true
    }
}

pub(super) struct RenderState {
    pub(super) document: ParsedDocument,
    /// CSS supplied through `Session::render`.
    ///
    /// This source is session-scoped and is not replaced by a document reload.
    pub(super) session_css: SharedSourceText,
    /// CSS extracted from the document currently being rendered. A whole
    /// document replacement updates this source before rules are rebuilt.
    pub(super) document_css: SharedSourceText,
    /// CSS injected via `innerHTML` `<style>` elements, appended to the
    /// session stylesheet incrementally.
    pub(super) extra_css: String,
    pub(super) css_rules: Arc<CssRule>,
    /// The dynamic pseudo-classes the stylesheet uses, computed once at rule
    /// build time; `Hover` / `Focus` commands test membership in O(1) and are
    /// skipped entirely when the stylesheet has no matching rule (browser-like
    /// no-op).
    pub(super) pseudo_rules: PseudoClassSet,
    pub(super) term_layout: Option<TermLayout>,
    /// Terminal title: install the document `<title>` once per session (or
    /// per replaced document). crossterm offers no title getter, so the
    /// previous title is intentionally left untouched on quit.
    pub(super) title_set: bool,
    /// The session interaction state driving the dynamic pseudo-classes:
    /// `hovered` (under the pointer) and `focused` (focused text field),
    /// mirrored from the input loop / script thread for `RenderArena` and
    /// `Hover` / `Focus` re-cascades.
    pub(super) dynamic: DynamicState,
    /// Cached geometry of the latest layout pass: kept so `RenderCmd::Hover`
    /// / `RenderCmd::Focus` can re-style the document for CSS `:hover` /
    /// `:focus` rules and repaint without re-parsing or re-laying out.
    pub(super) geometry: Option<SharedGeometry>,
    /// Whether the current document contains any `position: fixed` box. When
    /// true, scrolling re-anchors the fixed boxes to the new scroll offset
    /// The document contains `position: fixed` / `sticky` boxes, so scrolling
    /// re-pins them (a targeted region repaint) so they stay on screen;
    /// without pinned boxes scrolling keeps the cheap canvas re-slice.
    pub(super) has_pinned: bool,
    /// Active page text selection (canvas-space endpoints + shape); `None`
    /// while no selection exists.
    pub(super) text_selection: Option<TextSel>,
    /// Text inside the current selection, refreshed whenever the selection
    /// moves; the input loop reads it on `Ctrl+C` to copy.
    pub(super) selection_text: Arc<Mutex<String>>,
    /// The `<select>` dropdown overlay currently painted (`None` = none).
    pub(super) open_select: Option<OpenSelect>,
    /// Shared with the input loop; controls which scrollbar chrome paints.
    pub(super) scrollbars_visible: SharedScrollbarVisibility,
    pub(super) max_scroll: Arc<AtomicU32>,
}

impl RenderState {
    /// Repaint one `<input>` field's rectangle with fresh edit state. The
    /// field's layout is width-fixed, so value / caret / focus never change
    /// geometry — only the field's cells are repainted.
    pub(super) fn handle_input<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        node_id: NodeId,
        value: CompactString,
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
        let rect = find_box_rect(&geo.boxes, node_id);
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
    pub(super) fn handle_select<W: Write>(&mut self, out: &mut BufWriter<W>, cmd: RenderCmd) {
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
                    let prev_select = find_box_rect(&geo.boxes, prev.node_id).unwrap_or(prev.rect);
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
                let Some(select_rect) = find_box_rect(&geo.boxes, node_id) else {
                    return;
                };
                let rect = select_overlay_rect(&options, &rows, select_rect);
                self.open_select = Some(OpenSelect {
                    node_id,
                    filter,
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
                let select_rect = find_box_rect(&geo.boxes, open.node_id).unwrap_or(open.rect);
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
    pub(super) fn apply_text_selection_cmd<W: Write>(
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
            Some(_) => crate::layout::extract_flow_text(&geo.arena, &geo.boxes, &raw_lines),
            None => String::new(),
        };
        if let Ok(mut guard) = self.selection_text.lock() {
            *guard = text;
        }
    }
}

//! Frame pipeline: re-parse / re-cascade / re-layout / re-paint / flush, plus
//! the parse and CSS-source helpers shared by the render thread and `run`.

use std::io::{BufWriter, Write, stdout};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use crossterm::execute;
use indexmap::IndexMap;

use crate::events::Geometry;
use crate::html::css_parse::{
    self, CssRule, DynamicState, PseudoClass, css_into_node_with_state, css_pseudo_rule_set,
    reset_css_attr,
};
use crate::layout::Rect;
use crate::layout::layout::TermMap;
use crate::layout::layoutbox::{pin_fixed_boxes, pin_sticky_boxes};
use crate::tdom::node::{NodeArena, NodeId, NodeType};
use crate::tui::graph_api::{DirtyInfo, TermLayout};

use super::overlay::apply_select_overlay;
use super::region::{incremental_region, pinned_boxes_rects, pseudo_affected_region, union_rect};
use super::selection::{highlight_lines, lines_bounds};
use super::state::{RenderState, SELECTION_BG, cascade_document};

/// Flush one rendered frame from the session's `BufWriter`, skipping the
/// syscall entirely when the frame wrote nothing (an unchanged viewport).
pub(super) fn flush_frame<W: Write>(out: &mut BufWriter<W>) {
    if out.buffer().is_empty() {
        return;
    }
    if let Err(e) = out.flush() {
        log::error!("Failed to flush stdout: {}", e);
    }
}

pub(super) fn combined_css_source(embedded_css: &str, css: &str) -> String {
    if css.is_empty() {
        embedded_css.to_string()
    } else {
        format!("{}\n{}", embedded_css, css)
    }
}

/// Parse a full CSS source string into rules (empty source → empty rules).
pub(super) fn parse_css_source(source: &str) -> CssRule {
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

impl RenderState {
    /// Re-layout, re-paint, publish the geometry and flush a frame.
    ///
    /// Shared by `Redraw` (same document) and `RenderHtml` (new document).
    /// `out` is generic so tests can drive the pipeline against an in-memory
    /// writer.
    pub(super) fn redraw<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        scroll_y: u32,
        zoom: f32,
    ) {
        let (root_id, arena) = cascade_document(&self.document, &self.css_rules);
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
        // Whether the document contains any `position: fixed` box: scrolling
        // re-pins fixed / sticky boxes to the new scroll offset (a targeted
        // repaint) instead of the cheap canvas re-slice.
        self.has_pinned = !pinned_boxes_rects(&boxes).is_empty();

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
        // Re-apply an open dropdown overlay (a repaint would clear it). The
        // overlay re-anchors against the FRESH geometry first: this pass may
        // have re-laid-out the document (moved the field, changed the option
        // list), and painting the rect cached at open time would float the
        // dropdown at a stale position. A field that no longer produces a box
        // (document replaced / hidden) closes the overlay instead of painting
        // it into nowhere.
        let overlay_alive = match self.open_select.as_mut() {
            Some(open) => {
                let alive = open.reanchor(&geo.arena, &geo.boxes);
                if !alive {
                    self.open_select = None;
                }
                alive
            }
            None => false,
        };
        if overlay_alive {
            if let Some(open) = &self.open_select {
                apply_select_overlay(tl, open);
            }
        } else {
            tl.open_select = None;
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
    pub(super) fn render_arena<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        arena: &Arc<NodeArena>,
        root_id: NodeId,
        scroll_y: u32,
        zoom: f32,
        dirty: DirtyInfo,
        state_changed: bool,
    ) {
        // `innerHTML`-injected `<style>` CSS must extend the session
        // stylesheet before the cascade. The added rules may restyle any node
        // in the tree, so the incremental repaint is disabled for this frame.
        let css_changed = !dirty.injected_css.is_empty();
        if css_changed {
            self.apply_injected_css(&dirty.injected_css);
        }
        let source_arena = Arc::clone(arena);
        let mut arena = (*source_arena).clone();
        reset_css_attr(&mut arena, root_id);
        css_into_node_with_state(&mut arena, root_id, &self.css_rules, &self.dynamic);
        self.document.arena = source_arena;
        self.document.root_id = root_id;
        let dirty = if state_changed || css_changed {
            None
        } else {
            Some(&dirty)
        };
        self.layout_and_paint(out, geo_tx, term_map, arena, root_id, scroll_y, zoom, dirty);
    }

    /// Replace the session's injected-CSS set with `injected` (host ? css)
    /// and re-parse the combined stylesheet ? but only when the set actually
    /// changed: a timer tick that re-injects identical styles rebuilds the
    /// combined string and compares, without re-parsing.
    ///
    /// The host-keyed replacement tracks the document's *live* injected
    /// styles. The previous append-only accumulation grew without bound and
    /// re-parsed `base + full history` on every injected write (O(n?) under
    /// refresh churn); now both the storage and each re-parse are bounded by
    /// the page's actual style content.
    fn apply_injected_css(&mut self, injected: &IndexMap<NodeId, String>) {
        let mut extra = String::new();
        for css in injected.values() {
            if css.trim().is_empty() {
                continue;
            }
            extra.push_str(css);
            extra.push('\n');
        }
        if extra == self.extra_css {
            return;
        }
        self.extra_css = extra;
        self.rebuild_css_rules();
    }

    /// Re-parse `session_css + document_css + extra_css`.
    fn rebuild_css_rules(&mut self) {
        let mut combined = combined_css_source(&self.document_css, &self.session_css);
        if !self.extra_css.trim().is_empty() {
            combined.push('\n');
            combined.push_str(&self.extra_css);
        }
        let rules = parse_css_source(&combined);
        self.pseudo_rules = css_pseudo_rule_set(&rules);
        self.css_rules = Arc::new(rules);
    }

    /// Drop interaction state whose node ids belonged to a replaced document.
    fn reset_document_interaction(&mut self) {
        self.text_selection = None;
        self.open_select = None;
        self.dynamic = DynamicState::default();
        if let Ok(mut guard) = self.selection_text.lock() {
            guard.clear();
        }
        self.title_set = false;
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
    pub(super) fn repaint_dynamic<W: Write>(
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
        // The hovered / focused id may be stale: the element it names can be
        // removed by JS between the hit map's snapshot and this repaint (the
        // music-demo quick-click race). Filter both against the cached
        // arena — a stale id restyles nothing, and the ancestor-chain walk
        // below must never see a key the arena does not hold.
        let in_geo = |id: Option<NodeId>| id.filter(|id| geo.arena.contains_key(*id));
        let prev = in_geo(prev);
        let new = in_geo(new);
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

    /// Re-anchor `position: fixed` / `sticky` boxes to the current scroll
    /// offset and repaint the cells they occupy, then re-publish the geometry.
    ///
    /// The cached boxes were pinned to the *previous* scroll; this moves them
    /// by the scroll delta (document coordinates) so the canvas slice shows
    /// them at their constant screen position. Only the union of the old and
    /// new pinned areas is repainted (a small region), and the fresh geometry
    /// is forwarded to the hit-test thread so hit regions stay in sync. Backs
    /// the scroll-aware path of [`RenderCmd::Rerender`] when the document has
    /// pinned boxes; the caller renders the frame afterwards.
    pub(super) fn pin_fixed_and_repaint(&mut self, geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>) {
        let Some(tl) = self.term_layout.as_mut() else {
            return;
        };
        let Some(geo) = self.geometry.clone() else {
            return;
        };
        let old_rects = pinned_boxes_rects(&geo.boxes);
        let mut boxes = geo.boxes.clone();
        let viewport = Rect::new(
            0,
            tl.scroll_y as usize,
            tl.columns as usize,
            tl.rows as usize,
        );
        pin_fixed_boxes(&mut boxes, viewport);
        pin_sticky_boxes(&mut boxes, viewport);
        let new_rects = pinned_boxes_rects(&boxes);
        let fresh = Arc::new(Geometry {
            arena: geo.arena.clone(),
            boxes,
            root_id: geo.root_id,
        });
        self.geometry = Some(Arc::clone(&fresh));
        if let Some(tx) = geo_tx
            && let Err(e) = tx.send(Arc::clone(&fresh))
        {
            log::debug!("Hit-test channel closed: {}", e);
        }
        let Some(region) = old_rects.into_iter().chain(new_rects).reduce(union_rect) else {
            return;
        };
        tl.repaint_region(&fresh.arena, &fresh.boxes, region);
        // The repaint reset the region; re-apply the active overlays on top.
        if let Some(sel) = &self.text_selection {
            let lines = highlight_lines(&geo.arena, &geo.boxes, sel, tl.columns as u32);
            tl.apply_text_selection(&lines, SELECTION_BG);
        }
        // The dropdown re-anchors first: the re-pin above may have moved a
        // fixed / sticky field the overlay hangs from, so the cached rect
        // would float.
        if let Some(open) = self.open_select.as_mut()
            && open.reanchor(&fresh.arena, &fresh.boxes)
        {
            apply_select_overlay(tl, open);
        }
    }

    /// Replace the session document and fully re-render it, keeping the
    /// current scroll position / zoom.
    ///
    /// Backs `RenderCmd::RenderHtml`. Script-side replacements are intercepted
    /// by `ScriptEngine::reload` (the engine stays the document's single
    /// source of truth); this render-thread path remains only for direct API
    /// use and as a fallback when an in-engine reload fails.
    pub(super) fn render_html<W: Write>(
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
        self.document = {
            let mut parser = crate::html::html_parse::Parser::new(&html);
            parser.parse_with_scripts()
        };
        self.document_css = Arc::clone(&self.document.css);
        // A fresh document discards injected CSS and interaction state tied
        // to the replaced arena's node ids.
        self.extra_css = String::new();
        self.reset_document_interaction();
        self.rebuild_css_rules();
        self.redraw(out, geo_tx, term_map, scroll_y, zoom);
    }

    /// Replace session-level CSS and fully redraw the same document.
    ///
    /// Unlike `RenderDocument`, this does not reset interaction state: node
    /// ids and viewport state still belong to the live document.
    pub(super) fn reload_css<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        css: Arc<str>,
    ) {
        self.session_css = css;
        self.rebuild_css_rules();
        let (scroll_y, zoom) = match &self.term_layout {
            Some(tl) => (tl.scroll_y, tl.zoom),
            None => (0, 1.0),
        };
        self.redraw(out, geo_tx, term_map, scroll_y, zoom);
    }

    /// Render a whole-document replacement produced by `ScriptEngine::reload`.
    ///
    /// The arena and document CSS arrive from the script engine, preserving
    /// the single-source-of-truth rule: this thread never re-parses hot-reload
    /// HTML. `dirty.injected_css` may contain styles inserted by scripts that
    /// ran during the reload; those are applied after the document source is
    /// replaced. Scroll and zoom are viewport state, so they survive.
    pub(super) fn render_document<W: Write>(
        &mut self,
        out: &mut BufWriter<W>,
        geo_tx: &Option<mpsc::Sender<Arc<Geometry>>>,
        term_map: &TermMap,
        arena: Arc<NodeArena>,
        root_id: NodeId,
        document_css: Arc<str>,
        scroll_y: u32,
        zoom: f32,
        dirty: DirtyInfo,
    ) {
        self.document.arena = arena;
        self.document.root_id = root_id;
        self.document.css = Arc::clone(&document_css);
        self.document_css = document_css;
        self.extra_css = String::new();
        self.reset_document_interaction();
        self.rebuild_css_rules();
        if !dirty.injected_css.is_empty() {
            self.apply_injected_css(&dirty.injected_css);
        }
        self.redraw(out, geo_tx, term_map, scroll_y, zoom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::html_parse::Parser;
    use crossterm::style::Color;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU32;

    fn state(
        document: crate::html::html_parse::ParsedDocument,
        session_css: Arc<str>,
    ) -> RenderState {
        let document_css = Arc::clone(&document.css);
        let rules = parse_css_source(&combined_css_source(&document_css, &session_css));
        let pseudo_rules = css_pseudo_rule_set(&rules);
        RenderState {
            document,
            session_css,
            document_css,
            extra_css: String::new(),
            css_rules: Arc::new(rules),
            pseudo_rules,
            term_layout: None,
            title_set: false,
            geometry: None,
            has_pinned: false,
            dynamic: DynamicState::default(),
            text_selection: None,
            selection_text: Arc::new(Mutex::new(String::new())),
            open_select: None,
            max_scroll: Arc::new(AtomicU32::new(0)),
        }
    }

    #[test]
    fn reload_css_replaces_session_css_without_replacing_document_css() {
        let mut parser = Parser::new("<p>same</p>");
        let document = parser.parse_with_scripts();
        let mut render_state = state(document, Arc::from("p { color: red; }"));

        let mut out = BufWriter::new(Vec::new());
        let term_map = TermMap {
            columns: 80,
            rows: 24,
        };
        render_state.reload_css(&mut out, &None, &term_map, Arc::from("p { color: blue; }"));

        let rule = render_state
            .css_rules
            .0
            .first()
            .expect("session CSS contributes a rule");
        assert_eq!(
            rule.normal.get_color(),
            Some(Color::Rgb { r: 0, g: 0, b: 255 })
        );
        assert_eq!(render_state.session_css, Arc::from("p { color: blue; }"));
    }

    #[test]
    fn render_document_rebuilds_rules_from_the_new_document_css() {
        let mut old_parser = Parser::new("<p>old</p>");
        let old_document = old_parser.parse_with_scripts();
        let mut render_state = state(old_document, Arc::from(""));

        let mut parser = Parser::new("<style>p { color: red; }</style><p id=\"fresh\">fresh</p>");
        let document = parser.parse_with_scripts();
        let arena = Arc::clone(&document.arena);
        let root_id = document.root_id;
        let document_css = Arc::clone(&document.css);

        let mut out = BufWriter::new(Vec::new());
        let term_map = TermMap {
            columns: 80,
            rows: 24,
        };
        render_state.render_document(
            &mut out,
            &None,
            &term_map,
            arena,
            root_id,
            document_css,
            0,
            1.0,
            DirtyInfo::default(),
        );

        assert_eq!(render_state.document.css, render_state.document_css);
        let rule = render_state
            .css_rules
            .0
            .first()
            .expect("the reloaded document contributes a rule");
        assert_eq!(
            rule.normal.get_color(),
            Some(Color::Rgb { r: 255, g: 0, b: 0 }),
            "document CSS must be rebuilt after reload"
        );
    }
}

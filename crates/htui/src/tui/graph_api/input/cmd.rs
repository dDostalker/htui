use std::sync::Arc;

use compact_str::CompactString;
use indexmap::IndexMap;

use crate::html::css_parse::DynamicState;
use crate::source::SharedSourceText;
use crate::tdom::node::NodeId;

/// Shape of the page text selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Left-drag: continuous text between the anchor and the current point,
    /// spanning whole lines in between (browser-like reading order).
    Flow,
    /// Right-drag: rectangular column selection (vim `Ctrl+V` style).
    Block,
}

/// DOM mutations to repaint on the next `RenderArena` frame.
///
/// The script engine records the session-local node ids whose content or
/// attributes JS changed since the last render. When the set is non-empty the
/// render thread can try a region-limited repaint (re-layout the tree, then
/// repaint only the affected rectangles) instead of clearing and re-painting
/// the whole canvas; the rect comparison against the cached layout decides
/// whether that path is safe and falls back to a full repaint otherwise.
/// `injected_css` carries the live injected-CSS set (see the field docs) so
/// the render thread can rebuild the session stylesheet before re-cascading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirtyInfo {
    pub nodes: Vec<crate::tdom::node::NodeId>,
    /// CSS extracted from `<style>` / `<link rel=stylesheet>` elements
    /// inserted through `innerHTML`, keyed by the host element that received
    /// the write. Rewriting a host *replaces* its entry — the browser drops
    /// the host's old `<style>` elements — so this set tracks the document's
    /// live injected styles instead of accumulating an unbounded history.
    pub injected_css: IndexMap<crate::tdom::node::NodeId, String>,
}

impl DirtyInfo {
    /// True when no DOM mutation is pending (nothing to repaint).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.injected_css.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum RenderCmd {
    Rerender {
        scroll_y: u32,
    },
    Redraw {
        scroll_y: u32,
        zoom: f32,
    },
    /// Render a supplied DOM arena snapshot (the script engine's live arena
    /// after JS mutations), cascading styles and re-laying-out without
    /// re-parsing HTML. `dynamic` carries the session interaction state (the
    /// `:hover` target and the focused field), so the re-cascade re-applies
    /// both highlights in the same pass (no separate `Hover` / `Focus`
    /// command needed). `dirty` lists the nodes JS mutated since the previous
    /// frame, letting the render thread repaint only the affected rectangles
    /// when the re-layout leaves every box rect unchanged.
    RenderArena {
        arena: Arc<crate::tdom::node::NodeArena>,
        root_id: crate::tdom::node::NodeId,
        scroll_y: u32,
        zoom: f32,
        dynamic: DynamicState,
        dirty: DirtyInfo,
    },
    /// Render a whole-document replacement supplied by the script engine.
    ///
    /// The HTML was already re-parsed
    /// by `ScriptEngine::reload`, so this command carries the live arena and
    /// the new document-embedded CSS instead of asking the render thread to
    /// create a second source of truth. Interaction state is reset because the
    /// old node ids belong to the replaced arena.
    RenderDocument {
        arena: Arc<crate::tdom::node::NodeArena>,
        root_id: crate::tdom::node::NodeId,
        document_css: SharedSourceText,
        scroll_y: u32,
        zoom: f32,
        dirty: DirtyInfo,
    },
    /// Replace session-level CSS and fully re-render the same document.
    ///
    /// This is a stateful reload, not a snapshot: it can be ordered before or
    /// after a `RenderDocument`, so both a CSS save and an HTML save in the
    /// same batch are applied.
    ReloadCss {
        css: SharedSourceText,
    },
    /// Re-style the cached document for the CSS `:hover` pseudo-class with the
    /// given session-local node (`None` clears any hover), then repaint. With
    /// no `:hover` rules in the stylesheet this repaints to the same pixels.
    Hover {
        node_id: Option<NodeId>,
    },
    /// Re-style the cached document for the CSS `:focus` pseudo-class with the
    /// given focused text field (`None` clears any focus highlight), then
    /// repaint. Pushed by the input loop whenever the focus moves; with no
    /// `:focus` rules in the stylesheet this repaints to the same pixels.
    Focus {
        node_id: Option<NodeId>,
    },
    /// Repaint one `<input>` field with fresh edit state (value / caret /
    /// focus). The layout is unchanged — the field's rectangle is fixed — so
    /// only that rectangle is repainted.
    RenderInput {
        node_id: NodeId,
        value: CompactString,
        cursor: usize,
        focused: bool,
        selection: Option<(usize, usize)>,
    },
    /// Update one native scroll container and re-layout. Stateful because the
    /// offsets are interaction history, not a DOM mutation.
    RenderScroll {
        node_id: NodeId,
        scroll_y: u32,
    },
    /// Glide the page viewport or one native scroll container to `to` with an
    /// ease-out animation. Intercepted by the run loop (never forwarded to the
    /// render thread): the animation lives on the input side and emits one
    /// viewport command (`Rerender` / `RenderScroll`) per frame while it
    /// runs. `target` picks the page viewport or the scroll container.
    ScrollAnimate {
        target: super::scroll_anim::ScrollTarget,
        to: u32,
    },
    /// JS `el.focus()` — focus the text field `node_id`. Intercepted by the
    /// input loop (never forwarded to the render thread): it seeds the edit
    /// session and routes the `blur` / `focus` events back through the
    /// script-message path, so the focus state stays owned by the input side.
    FocusRequest {
        node_id: NodeId,
    },
    /// JS `el.blur()` — remove focus from `node_id` if it is the focused
    /// field (no-op otherwise, matching the browser). Intercepted by the
    /// input loop like [`RenderCmd::FocusRequest`](Self::FocusRequest).
    BlurRequest {
        node_id: NodeId,
    },
    /// Open the dropdown overlay for `node_id`: the render thread resolves the
    /// options (a `<select>`'s children, or an `<input list>`'s `<datalist>`
    /// filtered by `filter`), paints the overlay below the field and
    /// recolours the field's frame. `filter` is `None` for a plain `<select>`.
    SelectOpen {
        node_id: NodeId,
        filter: Option<CompactString>,
    },
    /// Close the dropdown overlay for `node_id` (clears the painted overlay
    /// and restores the field's frame).
    SelectClose {
        node_id: NodeId,
    },
    /// Move the dropdown's highlighted row to `highlight` (keyboard
    /// navigation) and repaint only the overlay.
    SelectMove {
        node_id: NodeId,
        highlight: usize,
    },
    /// Update the page text selection (canvas-space anchor and current
    /// endpoint; either `None` clears it). The render thread repaints the
    /// affected region with the selection highlight (flow or block shape) and
    /// refreshes the text extractable from it (read on `Ctrl+C` to copy).
    TextSelection {
        anchor: Option<(u32, u32)>,
        end: Option<(u32, u32)>,
        /// Selection shape; ignored when `anchor` / `end` are `None`.
        mode: SelectionMode,
    },
    Quit,
}

impl RenderCmd {
    /// Refresh the interaction state a `RenderArena` snapshot carries.
    ///
    /// The arena's `dynamic` is captured when its DOM snapshot is folded,
    /// while the focus is owned by the input loop and can move later in the
    /// same batch (`el.focus()` requests are expanded after the fold). Both
    /// the input loop (before sending) and the render thread's drain (when a
    /// `Focus` delta lands behind a pending arena) stamp the batch-final
    /// value through this method so the arena render paints the state the
    /// batch ends with. Non-arena commands are unaffected.
    pub fn refresh_dynamic_focus(&mut self, focused: Option<NodeId>) {
        if let RenderCmd::RenderArena { dynamic, .. } = self {
            dynamic.focused = focused;
        }
    }
}

/// Merge a later same-document dirty set into a pending document snapshot.
fn merge_dirty(target: &mut DirtyInfo, source: DirtyInfo) {
    target.nodes.extend(source.nodes);
    target.injected_css.extend(source.injected_css);
}

/// One render thread's drained command batch, classified by merge semantics.
///
/// The channel accumulates commands across input-loop batches while the
/// render thread finishes the previous frame, so the drain must decide per
/// command class what survives:
///
/// - Stateful, must-run-in-order commands — [`RenderCmd::RenderInput`] (a
///   focus switch queues blur + focus for two fields; dropping either leaves
///   a stale caret) and the `Select*` family (they mutate the render
///   thread's dropdown state, so a `SelectClose` must not be dropped in
///   favour of the [`RenderCmd::RenderArena`] its `change` handler
///   triggers) — are kept in arrival order and all run.
/// - Snapshot commands (`Rerender` / `Redraw` / `RenderArena` /
///   `RenderDocument` / `Quit`) each carry a complete final state, so a later
///   one safely supersedes an earlier one; only the last of the batch is
///   kept. Two exceptions preserve a richer snapshot the command cannot
///   reproduce: a `Rerender` / `Redraw` behind a pending `RenderArena` folds
///   its scroll / zoom into the arena or document snapshot instead of
///   replacing it (the arena carries DOM mutations a pure viewport command
///   must not drop).
/// - [`RenderCmd::TextSelection`] is *incremental*: its repaint region
///   derives from the previously applied selection, so superseding it with
///   a same-batch snapshot (a `RenderArena` triggered by a timer / hover
///   mid-drag) would leave the render thread re-applying the stale
///   highlight and make the next selection jump the edge forward — the
///   visible flicker. It is deferred instead and applied after the snapshot
///   command, on the fresh canvas.
/// - A [`RenderCmd::Hover`] arriving behind a pending *same-document*
///   snapshot (`RenderArena` / `Redraw` / `Rerender`) is likewise deferred:
///   superseding the snapshot would drop a resize re-layout or the DOM
///   mutations an arena carries (a resize while the pointer moves, an
///   animated widget under the pointer). The deferred hover repaint
///   re-applies the newer highlight on top. Behind a pending whole-document
///   snapshot it is dropped: the hover target belongs to the replaced document.
/// - A [`RenderCmd::Focus`] arriving behind a pending `RenderArena` *merges
///   into that arena's [`DynamicState`]* (the same pass paints the DOM
///   mutations and the newest focus); behind a pending `Redraw` / `Rerender`
///   it is deferred like a hover; behind a pending whole-document snapshot it
///   dropped (the focused field belonged to the replaced document).
#[derive(Debug, Default)]
pub struct DrainedBatch {
    inputs: Vec<RenderCmd>,
    scrolls: Vec<RenderCmd>,
    css_reloads: Vec<RenderCmd>,
    selects: Vec<RenderCmd>,
    text_selection: Option<RenderCmd>,
    hover: Option<RenderCmd>,
    focus: Option<RenderCmd>,
    snapshot: Option<RenderCmd>,
    quit: bool,
}

impl DrainedBatch {
    /// Create an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Classify and absorb one command — the batch's first (received with
    /// `recv`) or one drained afterwards (`try_recv`).
    pub fn absorb(&mut self, cmd: RenderCmd) {
        match cmd {
            RenderCmd::RenderInput { .. } => self.inputs.push(cmd),
            RenderCmd::RenderScroll { .. } => self.scrolls.push(cmd),
            // Never forwarded (the run loop intercepts markers before the
            // channel) — dropped here so a leaked one cannot displace a real
            // snapshot.
            RenderCmd::ScrollAnimate { .. } => {}
            sel @ (RenderCmd::SelectOpen { .. }
            | RenderCmd::SelectClose { .. }
            | RenderCmd::SelectMove { .. }) => self.selects.push(sel),
            css @ RenderCmd::ReloadCss { .. } => self.css_reloads.push(css),
            other @ RenderCmd::TextSelection { .. } => {
                // Only the latest selection matters; earlier ones in the
                // same batch are intermediates.
                self.text_selection = Some(other);
            }
            other @ RenderCmd::Hover { .. } => match self.snapshot.as_ref() {
                // A same-document snapshot is pending: defer the hover so
                // the newer highlight rides on top of it instead of
                // replacing it (superseding would drop a resize re-layout or
                // the DOM mutations an arena carries — see the type docs).
                Some(
                    RenderCmd::RenderArena { .. }
                    | RenderCmd::Redraw { .. }
                    | RenderCmd::Rerender { .. },
                ) => self.hover = Some(other),
                // The document was replaced: the hover target belonged to the
                // old document and must neither style nor displace the new one.
                Some(RenderCmd::RenderDocument { .. }) => {}
                // No snapshot pending (or the quit flag is already latched and
                // the snapshot is the quit): the hover repaint itself becomes
                // the batch's snapshot.
                _ if !self.quit => self.snapshot = Some(other),
                _ => {}
            },
            RenderCmd::Focus { node_id } => match self.snapshot.as_ref() {
                // Merge into the pending arena's interaction state: the same
                // pass paints the DOM mutations and the newest focus.
                Some(RenderCmd::RenderArena { .. }) => {
                    if let Some(snapshot) = self.snapshot.as_mut() {
                        snapshot.refresh_dynamic_focus(node_id);
                    }
                }
                // A same-document viewport snapshot is pending: defer the
                // focus delta so it re-styles the fresh frame instead of
                // replacing it.
                Some(RenderCmd::Redraw { .. } | RenderCmd::Rerender { .. }) => {
                    self.focus = Some(RenderCmd::Focus { node_id });
                }
                // The document was replaced: the focused field belonged to
                // the old document.
                Some(RenderCmd::RenderDocument { .. }) => {}
                _ if !self.quit => self.snapshot = Some(RenderCmd::Focus { node_id }),
                _ => {}
            },
            RenderCmd::Rerender { scroll_y } => match self.snapshot.as_mut() {
                // Fold the newer scroll offset into a pending arena render:
                // the arena carries the DOM mutations, the rerender only
                // moves the viewport.
                Some(
                    RenderCmd::RenderArena {
                        scroll_y: arena_scroll,
                        ..
                    }
                    | RenderCmd::RenderDocument {
                        scroll_y: arena_scroll,
                        ..
                    },
                ) => {
                    *arena_scroll = scroll_y;
                }
                _ => self.snapshot = Some(RenderCmd::Rerender { scroll_y }),
            },
            RenderCmd::Redraw { scroll_y, zoom } => match self.snapshot.as_mut() {
                // Fold the newer viewport state into a pending arena render
                // (a resize / zoom must not drop the DOM mutations).
                Some(
                    RenderCmd::RenderArena {
                        scroll_y: arena_scroll,
                        zoom: arena_zoom,
                        ..
                    }
                    | RenderCmd::RenderDocument {
                        scroll_y: arena_scroll,
                        zoom: arena_zoom,
                        ..
                    },
                ) => {
                    *arena_scroll = scroll_y;
                    *arena_zoom = zoom;
                }
                _ => self.snapshot = Some(RenderCmd::Redraw { scroll_y, zoom }),
            },
            RenderCmd::RenderArena {
                arena,
                root_id,
                scroll_y,
                zoom,
                dynamic,
                dirty,
            } => match self.snapshot.as_mut() {
                // A reload response can be followed in the same render batch
                // by another script reply (a timer, fetch, or focus event).
                // Keep the document snapshot: it carries the new CSS source,
                // while the newer arena carries the latest DOM mutations.
                Some(RenderCmd::RenderDocument {
                    arena: doc_arena,
                    root_id: doc_root,
                    scroll_y: doc_scroll,
                    zoom: doc_zoom,
                    dirty: doc_dirty,
                    ..
                }) => {
                    *doc_arena = arena;
                    *doc_root = root_id;
                    *doc_scroll = scroll_y;
                    *doc_zoom = zoom;
                    merge_dirty(doc_dirty, dirty);
                }
                _ if !self.quit => {
                    self.snapshot = Some(RenderCmd::RenderArena {
                        arena,
                        root_id,
                        scroll_y,
                        zoom,
                        dynamic,
                        dirty,
                    });
                }
                _ => {}
            },
            other => {
                self.quit |= matches!(other, RenderCmd::Quit);
                self.snapshot = Some(other);
            }
        }
    }

    /// `true` when a `Quit` was absorbed — draining may stop early, and the
    /// snapshot arm exits the render loop.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Drain the `RenderInput` commands in arrival order.
    pub fn drain_inputs(&mut self) -> impl Iterator<Item = RenderCmd> {
        std::mem::take(&mut self.inputs).into_iter()
    }

    /// Drain the native scrollbar updates in arrival order.
    pub fn drain_scrolls(&mut self) -> impl Iterator<Item = RenderCmd> {
        std::mem::take(&mut self.scrolls).into_iter()
    }

    /// Drain session-CSS reload commands in arrival order.
    pub fn drain_css_reloads(&mut self) -> impl Iterator<Item = RenderCmd> {
        std::mem::take(&mut self.css_reloads).into_iter()
    }

    /// Drain the `Select*` commands in arrival order.
    pub fn drain_selects(&mut self) -> impl Iterator<Item = RenderCmd> {
        std::mem::take(&mut self.selects).into_iter()
    }

    /// Take the deferred page text selection (the latest of the batch).
    pub fn take_text_selection(&mut self) -> Option<RenderCmd> {
        self.text_selection.take()
    }

    /// Take the snapshot command the batch ends with.
    pub fn take_snapshot(&mut self) -> Option<RenderCmd> {
        self.snapshot.take()
    }

    /// Take the deferred `Hover` (re-applied after the snapshot render).
    pub fn take_hover(&mut self) -> Option<RenderCmd> {
        self.hover.take()
    }

    /// Take the deferred `Focus` (re-applied after the snapshot render).
    pub fn take_focus(&mut self) -> Option<RenderCmd> {
        self.focus.take()
    }
}

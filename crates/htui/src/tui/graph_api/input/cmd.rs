use std::sync::Arc;

use crate::html::css_parse::DynamicState;
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
/// `style_css` carries CSS injected via `innerHTML` `<style>` elements so the
/// render thread can extend the session stylesheet before re-cascading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirtyInfo {
    pub nodes: Vec<crate::tdom::node::NodeId>,
    /// CSS extracted from `<style>` / `<link rel=stylesheet>` elements inserted
    /// through `innerHTML` since the last render.
    pub style_css: String,
}

impl DirtyInfo {
    /// True when no DOM mutation is pending (nothing to repaint).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.style_css.is_empty()
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
    /// Replace the session document with `html` and fully re-render it,
    /// keeping the current scroll position / zoom. Script-path uses are
    /// intercepted by `ScriptEngine::reload` so the engine re-parses the
    /// document (single source of truth); the render-thread branch only
    /// handles direct API use / fallback.
    RenderHtml {
        html: Arc<str>,
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
        value: String,
        cursor: usize,
        focused: bool,
        selection: Option<(usize, usize)>,
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
        filter: Option<String>,
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

/// Coalesce a command batch before forwarding it to the render thread.
///
/// A `RenderArena` carries the interaction state (`hovered` / `focused`)
/// captured when its DOM snapshot was folded, so it supersedes any earlier
/// `Hover` / `Focus` in the batch. A `Focus` queued *behind* it is still
/// dropped: the render thread's batch merge keeps only the last command, and
/// `Focus` is not deferred like `Hover`, so letting it supersede the arena
/// would lose the DOM mutation. `Hover` commands behind a pending arena are
/// kept — the render thread defers them and applies the newer highlight on
/// top of the arena render, so dropping them here would freeze the stale
/// highlight (a clicked button staying `:hover`-highlighted after the pointer
/// leaves).
pub fn coalesce_batch(cmds: Vec<RenderCmd>) -> Vec<RenderCmd> {
    let mut merged: Vec<RenderCmd> = Vec::with_capacity(cmds.len());
    let mut arena_pending = false;
    for cmd in cmds {
        match cmd {
            RenderCmd::RenderArena { .. } => {
                arena_pending = true;
                merged.push(cmd);
            }
            RenderCmd::Focus { .. } if arena_pending => {
                // Dropped: the arena render re-cascades with `focused`, and
                // superseding would drop the arena's DOM mutations.
            }
            other => merged.push(other),
        }
    }
    merged
}

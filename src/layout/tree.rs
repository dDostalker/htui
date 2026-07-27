//! Layout tree construction.
//!
//! Walks the DOM through the [`DEFAULT_LAYOUT`](crate::layout::DEFAULT_LAYOUT)
//! registry, building a forest of [`LayoutBox`]es rooted at the document.
//! Block-level children of the document share a single vertical [`Cursor`],
//! so each block claims a deterministic row range.

use crossterm::style::Color;

use crate::{
    layout::{
        BoxConstraint, BoxEdges, Cursor, DEFAULT_LAYOUT, InlineRun, LayoutBox, Rect,
    },
    tdom::node::{Node, NodeArena, NodeId, NodeType, effective_font_size_cells},
    tui::graph_api::{FontStyle, block_style},
};

/// Font-size context propagated through layout.
///
/// `font_size_cells` is the parent's effective font-size (in terminal
/// cells). `root_font_size_cells` is the root document's font-size, used to
/// resolve `rem` lengths. Each layout function reads `font_size_cells` to
/// compute the element's own font-size via
/// [`effective_font_size_cells`], then forwards the result down to nested
/// children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontContext {
    pub font_size_cells: usize,
    pub root_font_size_cells: usize,
}

impl FontContext {
    /// Default root context: 1-cell font, 1-cell root font.
    pub const ROOT: FontContext = FontContext {
        font_size_cells: 1,
        root_font_size_cells: 1,
    };

    /// Build a child context whose `em` resolves against `child_cells` and
    /// whose `rem` still resolves against the original root.
    pub fn child(&self, child_cells: usize) -> FontContext {
        FontContext {
            font_size_cells: child_cells,
            root_font_size_cells: self.root_font_size_cells,
        }
    }
}

/// Build a layout forest for `root_id`'s block children.
///
/// `containing` is the root containing block (typically the full terminal
/// width). `cursor` tracks the next free row and is advanced in-place so
/// callers can chain multiple passes.
pub fn layout_tree(
    arena: &NodeArena,
    root_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Vec<LayoutBox> {
    let children: Vec<NodeId> = arena[root_id].children.clone();
    layout_children(arena, &children, containing, cursor, FontContext::ROOT)
}

/// Lay out a sequence of sibling node IDs under `containing`.
pub fn layout_children(
    arena: &NodeArena,
    children: &[NodeId],
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Vec<LayoutBox> {
    let mut boxes = Vec::new();
    for &child_id in children {
        if let Some(b) = dispatch(arena, child_id, containing, cursor, font_ctx) {
            boxes.push(b);
        }
    }
    boxes
}

/// Iterate the layout registry; first claim wins.
fn dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    for layout_fn in DEFAULT_LAYOUT {
        if let Some(b) = layout_fn(arena, node_id, containing, cursor, font_ctx) {
            return Some(b);
        }
    }
    log::debug!("No layout handler for {:?}", arena[node_id].node_type);
    None
}

/// Collect inline runs from a block's inline tree, preserving `<span>` style
/// boundaries.
///
/// Walks `node`'s direct children in document order. Each `Text` leaf becomes
/// one run that inherits the currently-active style. `<span>` boundaries push
/// a new style onto the stack: the span's own `font-weight` / `font-style` /
/// `text-decoration` flags are OR-ed into the inherited `FontStyle`, and the
/// span's own `color` / `background-color` override the inherited `fg` / `bg`
/// when present. Block-level children are skipped — they are dispatched as
/// their own `LayoutBox` by the layout registry, so folding them into the
/// parent's runs would either double-render them or wrap them in an
/// unintended inline context.
pub fn collect_runs(
    arena: &NodeArena,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
) -> Vec<InlineRun> {
    let mut runs = Vec::new();
    walk_runs(arena, node, fg, bg, style, &mut runs);
    runs
}

fn walk_runs(
    arena: &NodeArena,
    node: &Node,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    runs: &mut Vec<InlineRun>,
) {
    for child_id in node.child_ids() {
        let child = &arena[child_id];
        match &child.node_type {
            NodeType::Text(s) => {
                if !s.is_empty() {
                    runs.push(InlineRun::new(s.clone(), style, fg, bg));
                }
            }
            NodeType::Span => {
                // Span inherits parent's fg/bg unless explicit overrides exist.
                let span_fg = child.css_attr.get_color().or(fg);
                let span_bg = child.css_attr.get_bg_color().or(bg);
                let span_style = style | block_style(&child.css_attr);
                walk_runs(arena, child, span_fg, span_bg, span_style, runs);
            }
            // Block-level nested elements live in their own LayoutBox;
            // unknown inline tags are flattened by the HTML parser so they
            // never reach here as an unknown NodeType.
            _ => {}
        }
    }
}

/// Compute a block-level element's CSS width into a column count, clamped to
/// the containing block.
pub fn resolve_block_width(css_width: Option<usize>, containing_width: usize) -> usize {
    css_width.unwrap_or(containing_width).min(containing_width)
}

/// Compute the left offset for a block-level element inside its containing
/// block.
///
/// Mirrors CSS block-layout default: a block with an explicit `width` sits
/// flush against the containing block's left edge (i.e. `margin-left: 0`,
/// `margin-right: auto` only when both are auto — which would need a
/// `margin: auto` rule we don't currently parse). `text-align` is a
/// separate concern handled in the paint phase.
pub fn block_left(containing: BoxConstraint) -> usize {
    containing.x
}

/// Resolved outer geometry for a block-level element before children are
/// laid out.
///
/// `outer_x` / `outer_y` / `outer_width` describe the **border-box** — the
/// rectangle that gets painted (background, border glyphs) — and
/// `content_constraint` is the inner block passed to nested children,
/// already shrunk by `padding` and `border` so children's `width: 100%`
/// fills the content area.
#[derive(Debug, Clone, Copy)]
pub struct OuterGeometry {
    pub outer_x: usize,
    pub outer_y: usize,
    pub outer_width: usize,
    pub content_constraint: BoxConstraint,
}

/// Compute the outer (border-box) geometry for a block-level element given
/// its CSS edges and the chosen border-box width.
///
/// - `containing` is the parent's content-box
/// - `block_width` is the resolved CSS `width` for the border-box (i.e. the
///   rect width). It must already include padding + border — this helper
///   does not add them on top.
///
/// The returned `content_constraint` has its `x` shifted by `border.left` +
/// `padding.left` and its `width` reduced by the same totals on each side.
pub fn compute_outer_geometry(
    containing: BoxConstraint,
    block_width: usize,
    edges: BoxEdges,
    cursor_y: usize,
) -> OuterGeometry {
    let outer_x = containing.x + edges.margin.left;
    let outer_width = block_width;
    let outer_y = cursor_y;

    let inner_h = edges.inner_horizontal();
    let content_x = outer_x + edges.border.left.thickness() + edges.padding.left;
    let content_width = outer_width.saturating_sub(inner_h).max(1);
    let content_constraint = BoxConstraint {
        x: content_x,
        width: content_width,
    };

    OuterGeometry {
        outer_x,
        outer_y,
        outer_width,
        content_constraint,
    }
}

/// Place a border-box rectangle at `outer` by reserving `margin.top` rows
/// above and `margin.bottom` rows below on `cursor`.
///
/// Returns the row index where the border-box *top* sits and advances the
/// cursor past the bottom margin so the next sibling starts with the
/// correct vertical spacing. `content_height` is the number of rows the
/// nested children consumed (or `min_content_rows` for inline-only blocks
/// that haven't allocated a row of their own).
///
/// `margin.top` is **not** reserved by this helper — callers use
/// [`Cursor::skip`] before invoking it so the cursor sits exactly on the
/// border-box's top row when the rect is materialised. `margin.bottom` is
/// applied here.
pub fn reserve_border_box(
    cursor: &mut Cursor,
    outer_y_top: usize,
    edges: BoxEdges,
    content_height: usize,
    min_content_rows: usize,
) -> usize {
    // The border-box height always accommodates the minimum content area
    // even when there are no nested children — an empty `<p>` still owns
    // its line.
    let content_height = content_height.max(min_content_rows);
    let outer_height = content_height
        .saturating_add(edges.padding.top)
        .saturating_add(edges.padding.bottom)
        .saturating_add(edges.border.top.thickness())
        .saturating_add(edges.border.bottom.thickness());
    cursor.y = outer_y_top
        .saturating_add(outer_height)
        .saturating_add(edges.margin.bottom);
    outer_height
}

/// Build a border-box [`Rect`] from an [`OuterGeometry`] and the resolved
/// outer height.
pub fn build_border_rect(geom: OuterGeometry, outer_height: usize) -> Rect {
    Rect::new(geom.outer_x, geom.outer_y, geom.outer_width, outer_height)
}

/// Per-element differences layered on top of the shared block-layout pipeline.
///
/// All fields default to "no special behaviour" so the common case is just
/// `LayoutBlockOptions::default()`.
#[derive(Debug, Default, Clone)]
pub struct LayoutBlockOptions {
    /// Inline run prepended to the body runs (e.g. `# ` for `<h1>`, `• ` for
    /// `<li>`). The run uses the element's own fg / bg / font style so the
    /// body runs can layer additional styles on top.
    pub prefix: Option<InlineRun>,
    /// When `true`, the rect's background is filled even without an explicit
    /// CSS `background-color` — used by `<aside>` (solid stripe) and
    /// `<header>` / `<footer>` (chrome bands).
    pub fill_bg_default: bool,
    /// Trailing blank rows reserved below the rect (default 0). The paint
    /// phase skips these rows when filling the background. `<header>` /
    /// `<footer>` use 2 to leave breathing room before the next block.
    pub spacing_after: usize,
    /// Extra columns added to `containing.x` and subtracted from
    /// `containing.width` when computing the border-box rect (used by nested
    /// `<ul>` / `<ol>` to indent items per nesting level).
    pub indent: usize,
    /// `(x_offset, width_delta)` applied to the rect's content-box to
    /// derive the children's containing block. `<li>` uses this to shift
    /// nested children past the marker width — `width_delta` is negative
    /// so the inner width shrinks by the same amount.
    pub child_containing_shift: Option<(usize, isize)>,
    /// Marks the block as a chrome element (`<aside>`/`<header>`/`<footer>`)
    /// whose background always covers the full border-box regardless of
    /// inline content width or text-align. Also covers blocks that opted
    /// into the same semantics via `fill_bg_default` (currently just the
    /// chrome trio).
    pub chrome: bool,
}

/// Shared body of every block-level layout function.
///
/// Resolves the CSS `width`, padding / border / margin, and the inline
/// `runs`, then reserves the border-box rows for `node` and lays out its
/// block children. The element-specific differences are passed in via
/// `opts` so the boilerplate (collect_runs, skip margin-top, layout
/// children, reserve_border_box) lives in one place.
///
/// `font_ctx` carries the parent's effective font-size and the root
/// font-size so `em` / `rem` lengths inside the element resolve against the
/// active font-size context. The element's own font-size is computed via
/// [`effective_font_size_cells`] and forwarded to nested children so the
/// chain remains consistent.
///
/// Returns the assembled `LayoutBox`. The caller is still responsible for
/// checking that `node.node_type` matches the element it claims to lay out
/// (so this helper stays type-agnostic).
pub fn layout_block(
    arena: &NodeArena,
    node: &Node,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    opts: LayoutBlockOptions,
    font_ctx: FontContext,
) -> LayoutBox {
    let own_font_size_cells = effective_font_size_cells(
        arena,
        node_id,
        font_ctx.font_size_cells,
        font_ctx.root_font_size_cells,
    );
    let child_font_ctx = font_ctx.child(own_font_size_cells);

    let edges: BoxEdges = node.css_attr.get_box_edges(
        containing.width,
        own_font_size_cells,
        font_ctx.root_font_size_cells,
    );

    let available = containing
        .width
        .saturating_sub(opts.indent)
        .saturating_sub(edges.margin.horizontal());
    let block_width = resolve_block_width(
        node.css_attr
            .get_width(available, own_font_size_cells, font_ctx.root_font_size_cells),
        available,
    )
    .max(1);
    let x = block_left(containing) + opts.indent;

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    let fill_bg = block_bg.is_some() || opts.fill_bg_default;

    cursor.skip(edges.margin.top);
    let outer_y_top = cursor.y;

    let geom = compute_outer_geometry(
        BoxConstraint {
            x,
            width: block_width,
        },
        block_width,
        edges,
        outer_y_top,
    );

    let child_containing = match opts.child_containing_shift {
        Some((x_off, w_delta)) => BoxConstraint {
            x: geom.content_constraint.x.saturating_add(x_off),
            width: if w_delta >= 0 {
                geom
                    .content_constraint
                    .width
                    .saturating_add(w_delta as usize)
                    .max(1)
            } else {
                geom
                    .content_constraint
                    .width
                    .saturating_sub((-w_delta) as usize)
                    .max(1)
            },
        },
        None => geom.content_constraint,
    };
    let mut child_cursor = Cursor {
        y: geom.outer_y + edges.border.top.thickness() + edges.padding.top,
    };
    let block_children = node.children.clone();
    let children = layout_children(
        arena,
        &block_children,
        child_containing,
        &mut child_cursor,
        child_font_ctx,
    );

    let content_height = child_cursor.y.saturating_sub(
        geom.outer_y + edges.border.top.thickness() + edges.padding.top,
    );
    let outer_height = reserve_border_box(
        cursor,
        geom.outer_y,
        edges,
        content_height,
        own_font_size_cells.max(1),
    );

    let mut runs = body_runs;
    if let Some(prefix) = opts.prefix {
        runs.insert(0, prefix);
    }

    // `spacing_after` rows are reserved inside the rect (so paint can skip
    // them when filling the background) and also advance the cursor past
    // them so the next sibling starts with the correct vertical spacing.
    let rect_height = outer_height + opts.spacing_after;
    if opts.spacing_after > 0 {
        cursor.y = cursor.y.saturating_add(opts.spacing_after);
    }

    if opts.chrome || opts.fill_bg_default {
        LayoutBox::new_block_with_chrome(
            node_id,
            build_border_rect(geom, rect_height),
            edges,
            fill_bg,
            opts.spacing_after,
            runs,
            children,
            own_font_size_cells,
            opts.chrome,
        )
    } else {
        LayoutBox::new_block(
            node_id,
            build_border_rect(geom, rect_height),
            edges,
            fill_bg,
            opts.spacing_after,
            runs,
            children,
            own_font_size_cells,
        )
    }
}

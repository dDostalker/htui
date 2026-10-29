//! Layout pipeline.
//!
//! ```text
//! DOM → LayoutBox tree → Block grid
//! ```
//!
//! The two-phase pipeline is split as follows:
//!
//! - [`layoutbox`] defines the [`LayoutBox`] data model and its three-axis
//!   classification ([`BoxKind`], [`OuterDisplay`], [`FormattingContext`]).
//! - [`layoutbox::inner_formatting_context::flow`], [`flex`](self::layoutbox::inner_formatting_context::flex), and [`grid`](self::layoutbox::inner_formatting_context::grid)
//!   implement the three formatting contexts that build layout trees from a
//!   DOM. Each lives under
//!   [`layoutbox::inner_formatting_context`] and shares the same
//!   [`LayoutBox`] tree shape.
//! - [`paint`] walks a finished `Vec<LayoutBox>` and writes
//!   [`Block`](crate::graphics::Block) cells into the terminal canvas.
//! - [`layout`] (this file) glues everything together: [`DEFAULT_LAYOUT`] is
//!   the registry of layout functions consulted by the flow orchestrator.

use crate::{
    graphics::grapheme_width,
    html::css_parse::{TextAlign, UserSelect},
    layout::layout::TermMap,
    tdom::node::{NodeArena, NodeId, NodeType},
};
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

pub mod layout;
pub mod layoutbox;
pub mod paint;

pub use crate::html::box_edges::{Border, BorderSides, BorderStyle, BoxEdges, EdgeSizes};
pub use crate::html::css_parse::{FontContext, Viewport};
pub use layoutbox::inner_formatting_context;
pub use layoutbox::inner_formatting_context::{
    flex::layout_flex_dispatch, flow::layout_tree, flow::layout_tree_with_scroll_offsets,
};
pub use layoutbox::{
    BoxKind, ColumnLayout, Cursor, FormattingContext, GeneratedBox, GeneratedBoxKind,
    GeneratedPayload, InlineRun, LayoutBox, OuterDisplay, PaintStyle, Rect, ReplacedContent,
    ScrollbarLayout, flatten_contents_children, generate_box_tree, generate_node,
    layout_generated_box, layout_generated_tree,
};

/// Context propagated by the layout phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutContext<'a> {
    pub font: FontContext,
    pub viewport: Viewport,
    /// Session scroll offsets for native `overflow-y` containers.
    pub scroll_offsets: &'a HashMap<NodeId, u32>,
}

/// Empty offset table for callers that do not create scroll containers.
pub static NO_SCROLL_OFFSETS: std::sync::LazyLock<std::collections::HashMap<NodeId, u32>> =
    std::sync::LazyLock::new(std::collections::HashMap::new);

impl<'a> LayoutContext<'a> {
    /// Derive a child context with the given font size, keeping the viewport.
    pub fn child(self, child_font_size_cells: usize) -> Self {
        Self {
            font: self.font.child(child_font_size_cells),
            viewport: self.viewport,
            scroll_offsets: self.scroll_offsets,
        }
    }
}

/// Rebuild the text of a flow selection from the laid-out inline runs.
///
/// `lines` is the set of per-row column bands the selection covers
/// (`(row, col_start, col_end_exclusive)` in canvas coordinates), as produced
/// by the render thread's selection tracking. Unlike a per-cell canvas
/// extraction (which would insert a newline at every wrap point), this follows
/// the document's block structure: lines belonging to the same DOM block are
/// concatenated directly — a wrapped paragraph copies as one continuous
/// string, like a browser — and a newline is inserted only when the selection
/// crosses into another block.
///
/// Inline children (`<span>` / text nodes) are folded into an anonymous box
/// carrying the parent block's node id during generation and never produce a
/// `LayoutBox` of their own, so the `node_id` of every box carrying inline
/// runs is exactly its owning DOM block.
pub fn extract_flow_text(
    arena: &NodeArena,
    boxes: &[LayoutBox],
    lines: &[(u32, u32, u32)],
) -> String {
    let mut text_lines: Vec<FlowTextLine> = Vec::new();
    collect_flow_lines(arena, boxes, &mut text_lines);

    // Fragments: (row, first selected column, owning block, text).
    let mut frags: Vec<(u32, u32, NodeId, String)> = Vec::new();
    for &(sel_row, sel_cs, sel_ce) in lines {
        for tl in &text_lines {
            if tl.row != sel_row {
                continue;
            }
            let line_width: u32 = tl.runs.iter().map(|r| r.width_at(tl.font) as u32).sum();
            let line_end = tl.col_start.saturating_add(line_width);
            let lo = sel_cs.max(tl.col_start);
            let hi = sel_ce.min(line_end);
            if lo >= hi {
                continue;
            }
            let text = extract_line_text(tl.runs, tl.font, lo, hi, tl.col_start);
            if !text.is_empty() {
                frags.push((tl.row, lo, tl.node_id, text));
            }
        }
    }

    // Assemble in visual order: same block → concatenate, new block → newline.
    frags.sort_by_key(|&(row, col, _, _)| (row, col));
    let mut out = String::new();
    let mut prev_block: Option<NodeId> = None;
    for (_, _, node_id, text) in frags {
        if !out.is_empty() && prev_block != Some(node_id) {
            out.push('\n');
        }
        out.push_str(&text);
        prev_block = Some(node_id);
    }
    out
}

/// Resolve CSS `user-select` for a node, including the `auto` ancestor walk.
///
/// `user-select` is not inherited, but `auto` resolves its *used* value from
/// the nearest explicit non-`auto` declaration. This lets a `text` descendant
/// opt back in beneath `none`, while an `auto` descendant remains unselectable.
pub fn user_select_is_none(arena: &NodeArena, start: NodeId) -> bool {
    let mut current = Some(start);
    while let Some(node_id) = current {
        let Some(node) = arena.get(node_id) else {
            return false;
        };
        match node.css_attr.get_user_select() {
            UserSelect::None => return true,
            UserSelect::Text | UserSelect::All | UserSelect::Contain => return false,
            UserSelect::Auto => current = node.parent,
        }
    }
    false
}

/// Clip a flow selection's row bands down to the columns each row actually
/// covers with text.
///
/// The selection tracking works in whole rows (`col_start..canvas_width` for
/// full rows), but painting the highlight over an entire row would wash the
/// block's border, padding, indentation and trailing whitespace with the
/// selection background. This maps every input band onto the text columns of
/// the matching visual line (each overlapping text fragment becomes its own
/// band), so the highlight covers exactly the cells whose glyphs the copy
/// would yield — and nothing beyond the block's own text.
///
/// Block selections (right-drag rectangles) are *not* passed through this:
/// their highlight intentionally follows the dragged rectangle.
pub fn selection_text_lines(
    arena: &NodeArena,
    boxes: &[LayoutBox],
    lines: &[(u32, u32, u32)],
) -> Vec<(u32, u32, u32)> {
    let mut text_lines: Vec<FlowTextLine> = Vec::new();
    collect_flow_lines(arena, boxes, &mut text_lines);
    let mut out: Vec<(u32, u32, u32)> = Vec::new();
    for &(sel_row, sel_cs, sel_ce) in lines {
        for tl in &text_lines {
            if tl.row != sel_row {
                continue;
            }
            let line_width: u32 = tl.runs.iter().map(|r| r.width_at(tl.font) as u32).sum();
            let text_end = tl.col_start.saturating_add(line_width);
            let lo = sel_cs.max(tl.col_start);
            let hi = sel_ce.min(text_end);
            if lo < hi {
                out.push((sel_row, lo, hi));
            }
        }
    }
    out
}

/// One laid-out visual line of text, positioned exactly as the paint phase
/// draws it (same border / padding / glyph-row / wrap math).
struct FlowTextLine<'a> {
    node_id: NodeId,
    row: u32,
    col_start: u32,
    runs: &'a [InlineRun],
    font: usize,
}

/// Collect every visual text line of the layout with the paint phase's exact
/// geometry, so extraction sees the same cells the user sees.
fn collect_flow_lines<'a>(
    arena: &'a NodeArena,
    boxes: &'a [LayoutBox],
    out: &mut Vec<FlowTextLine<'a>>,
) {
    for lb in boxes {
        collect_box_lines(arena, lb, out);
        collect_flow_lines(arena, &lb.children, out);
    }
}

fn collect_box_lines<'a>(arena: &'a NodeArena, lb: &'a LayoutBox, out: &mut Vec<FlowTextLine<'a>>) {
    // Replaced controls own their terminal chrome and interactive values.
    // They are edited / copied through their control-specific paths, never as
    // ordinary text in a page flow selection.
    if matches!(lb.kind, crate::layout::BoxKind::Replaced(_))
        || user_select_is_none(arena, lb.node_id)
        || matches!(arena[lb.node_id].node_type, NodeType::Progress | NodeType::Meter)
    {
        return;
    }
    let Some(runs) = &lb.paint_style.runs else {
        return;
    };
    if runs.is_empty() {
        return;
    }
    let be = &lb.paint_style.box_edges;
    let font = lb.paint_style.font_size_cells.max(1);
    let line_h = lb.paint_style.line_height_cells.max(1);
    let align = arena[lb.node_id].css_attr.get_text_align();
    let glyph_row = if line_h > font {
        (line_h - font) / 2
    } else {
        font.saturating_sub(1) / 2
    };
    let content_x = lb
        .rect
        .x
        .saturating_add(be.border.left.thickness())
        .saturating_add(be.padding.left);
    let content_width = lb.rect.width.saturating_sub(be.inner_horizontal()).max(1);
    let first_row = lb
        .rect
        .y
        .saturating_add(be.border.top.thickness())
        .saturating_add(be.padding.top)
        .saturating_add(glyph_row);
    for (i, line_runs) in runs.iter().enumerate() {
        // Forced-break blank rows (`<br>`, `<pre>` newlines) carry no glyphs.
        if line_runs.is_empty() {
            continue;
        }
        let line_width: usize = line_runs.iter().map(|r| r.width_at(font)).sum();
        let line_width = line_width.min(content_width);
        let offset = match align {
            TextAlign::Left => 0,
            TextAlign::Center => content_width.saturating_sub(line_width) / 2,
            TextAlign::Right => content_width.saturating_sub(line_width),
        };
        out.push(FlowTextLine {
            node_id: lb.node_id,
            row: (first_row.saturating_add(i.saturating_mul(line_h))) as u32,
            col_start: (content_x.saturating_add(offset)) as u32,
            runs: line_runs,
            font,
        });
    }
}

/// Rectangular hit regions for inline elements that own their runs
/// (`<label>` / `<a>`), derived from the same geometry the paint phase draws
/// (see `collect_box_lines`). Each owned run's visual slice becomes a
/// region, so a click on the element's text resolves to that element — the
/// delegation target for `<label for>` / wrapped controls, and the click
/// target for inline links.
///
/// The runs carry the owning element's node id (see `layoutbox::inline`);
/// plain text runs have `node_id: None` and produce no region.
pub fn label_regions(arena: &NodeArena, boxes: &[LayoutBox]) -> Vec<(NodeId, Rect)> {
    let mut out = Vec::new();
    collect_label_regions(arena, boxes, &mut out);
    out
}

fn collect_label_regions(arena: &NodeArena, boxes: &[LayoutBox], out: &mut Vec<(NodeId, Rect)>) {
    for lb in boxes {
        if let Some(runs) = &lb.paint_style.runs {
            collect_box_label_regions(arena, lb, runs, out);
        }
        collect_label_regions(arena, &lb.children, out);
    }
}

fn collect_box_label_regions(
    arena: &NodeArena,
    lb: &LayoutBox,
    runs: &[Vec<InlineRun>],
    out: &mut Vec<(NodeId, Rect)>,
) {
    if runs.is_empty() {
        return;
    }
    let be = &lb.paint_style.box_edges;
    let font = lb.paint_style.font_size_cells.max(1);
    let line_h = lb.paint_style.line_height_cells.max(1);
    let align = arena[lb.node_id].css_attr.get_text_align();
    let glyph_row = if line_h > font {
        (line_h - font) / 2
    } else {
        font.saturating_sub(1) / 2
    };
    let content_x = lb
        .rect
        .x
        .saturating_add(be.border.left.thickness())
        .saturating_add(be.padding.left);
    let content_width = lb.rect.width.saturating_sub(be.inner_horizontal()).max(1);
    let first_row = lb
        .rect
        .y
        .saturating_add(be.border.top.thickness())
        .saturating_add(be.padding.top)
        .saturating_add(glyph_row);
    for (i, line_runs) in runs.iter().enumerate() {
        if line_runs.is_empty() {
            continue;
        }
        let line_width: usize = line_runs.iter().map(|r| r.width_at(font)).sum();
        let line_width = line_width.min(content_width);
        let offset = match align {
            TextAlign::Left => 0,
            TextAlign::Center => content_width.saturating_sub(line_width) / 2,
            TextAlign::Right => content_width.saturating_sub(line_width),
        };
        let row = first_row.saturating_add(i.saturating_mul(line_h));
        let mut col = content_x.saturating_add(offset);
        for run in line_runs {
            let w = run.width_at(font);
            if let Some(node_id) = run.node_id {
                out.push((node_id, Rect::new(col, row, w, 1)));
            }
            col = col.saturating_add(w);
        }
    }
}

/// Extract the column slice `[lo, hi)` of one visual line's runs, where the
/// columns are absolute canvas columns starting at `line_start`.
fn extract_line_text(runs: &[InlineRun], font: usize, lo: u32, hi: u32, line_start: u32) -> String {
    let font = font.max(1);
    let mut out = String::new();
    let mut col = line_start;
    for run in runs {
        let run_w = run.width_at(font) as u32;
        let run_lo = col.max(lo);
        let run_hi = col.saturating_add(run_w).min(hi);
        if run_lo < run_hi {
            out.push_str(&run_text_between(
                run,
                font,
                (run_lo - col) as usize,
                (run_hi - col) as usize,
            ));
        }
        col = col.saturating_add(run_w);
        if col >= hi {
            break;
        }
    }
    out
}

/// The graphemes of `run` occupying columns `[lo, hi)` (relative to the run's
/// own start), using the same per-grapheme slot width the paint phase applies.
fn run_text_between(run: &InlineRun, font: usize, lo: usize, hi: usize) -> String {
    let font = font.max(1);
    let mut out = String::new();
    let mut col = 0usize;
    for g in run.text.graphemes(true) {
        let slot = grapheme_width(g).max(font);
        if col >= hi {
            break;
        }
        if col.saturating_add(slot) > lo {
            out.push_str(g);
        }
        col += slot;
    }
    out
}

///
/// A child element resolves its own width / horizontal position relative to
/// its containing block, mirroring CSS containing-block semantics.
#[derive(Debug, Clone, Copy)]
pub struct BoxConstraint {
    pub x: usize,
    pub width: usize,
}

impl BoxConstraint {
    /// Creates a root constraint spanning the terminal width.
    pub fn root(term_map: &TermMap) -> Self {
        Self {
            x: 0,
            width: term_map.columns as usize,
        }
    }
}

/// Layout function signature for the two-phase pipeline.
///
/// Each registered function inspects `node_id`; returns `None` to skip
/// (passes through to the next entry), or `Some(LayoutBox)` to claim the node.
/// The builder in
/// [`layoutbox::inner_formatting_context::flow::layout_tree`] walks the
/// registry in order and stops at the first claimer. `font_ctx` carries the
/// parent's effective font-size (and the root font-size) so `em` / `rem`
/// lengths resolve against the active font-size context.
///
/// `pre_allocated_rect` lets the caller (currently only the flex layout) hand
/// the layout function a fixed outer rect, instead of computing one from the
/// shared `cursor`. The layout function MUST honor it when present — flex
/// items have their rectangle determined by the flex algorithm, not by the
/// block flow. `None` means "allocate the rect from the cursor yourself".
/// This mirrors the way browser layout engines let a formatting context
/// (Block Formatting Context, Flex Formatting Context, …) determine the
/// outer rect of a child and then call into the child's layout function with
/// that rect — the child does not recompute it.
pub type LayoutFn = fn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox>;

/// Layout function registry.
///
/// Each element type that needs custom layout declares a function here. The
/// dispatcher iterates this list and uses the first non-`None` result.
///
/// The geometry dispatcher handles replaced, flex, and table boxes before this
/// registry is reached. This registry therefore contains only normal-flow
/// element adapters retained for behavior and public-API compatibility.
///
/// Plain block elements (`<div>` / `<p>` / `<aside>` / …) are folded into
/// [`layout_standard_block`](inner_formatting_context::flow::layout_standard_block)
/// and its `NodeType → LayoutBlockOptions` table; the remaining entries are
/// the dynamically-optioned adapters (headings, lists, anchors) and the
/// table-element pass-throughs kept for API compatibility.
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[
    inner_formatting_context::flow::layout_standard_block,
    inner_formatting_context::flow::layout_hn,
    inner_formatting_context::flow::layout_title,
    inner_formatting_context::flow::layout_meta,
    inner_formatting_context::flow::layout_a,
    inner_formatting_context::flow::layout_hr,
    inner_formatting_context::flow::layout_li,
    inner_formatting_context::flow::layout_ol,
    inner_formatting_context::flow::layout_ul,
    inner_formatting_context::flow::layout_menu,
    inner_formatting_context::flow::layout_row_group,
    inner_formatting_context::flow::layout_td,
    inner_formatting_context::flow::layout_th,
    inner_formatting_context::flow::layout_tr,
    inner_formatting_context::flow::layout_progress,
    inner_formatting_context::flow::layout_summary,
    inner_formatting_context::flow::layout_inline_blockify,
    inner_formatting_context::flow::layout_generic,
];

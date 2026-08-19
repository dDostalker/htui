//! Paint phase.
//!
//! Walks a [`LayoutBox`] forest produced by
//! [`super::layoutbox::inner_formatting_context::flow::layout_tree`] (and
//! flex / grid trees) and writes styled cells into the
//! [`TermLayout`](crate::graphics::TermLayout) canvas. Reads no DOM
//! state beyond each box's `node_id`: every paint decision is derived from
//! the box's own `rect`, `kind`, `paint_style`, and the CSS attributes
//! stored on the source node.
//!
//! ## Submodules
//!
//! - [`block`]: painting for `BoxKind::Normal` — backgrounds, borders, inline
//!   runs.
//! - [`border`]: the border-glyph rasteriser, shared with `block`.
//! - [`image`]: rasterising for `BoxKind::Replaced(ReplacedContent::Image)`.
//! - [`text`]: the inline-run painter, shared with `block`.

mod block;
mod border;
mod button;
mod image;
mod input;
mod select;
mod text;
mod textarea;

use std::collections::HashMap;

use crossterm::style::Color;

use crate::{
    html::css_parse::{Position, Visibility},
    layout::{BoxKind, LayoutBox, Rect, ReplacedContent},
    tdom::node::{NodeArena, NodeId, NodeType},
    graphics::{Block, FontStyle, ImageCache, InputRenderState, TermLayout},
};

use self::block::paint_block;
use self::border::paint_borders;
use self::button::paint_button;
use self::image::paint_image;
use self::input::paint_input;
use self::select::paint_select;
use self::textarea::paint_textarea;

/// Paint a replaced control's shared chrome: the background fill across the
/// whole border-box (the CSS `background-color`, or [`Theme::control_bg`]
/// when unset), then the CSS `border` frame when one is declared. Returns the
/// content rect — the border-box shrunk by the border thickness — that the
/// caller draws its value / label into; with no border this is the full box.
pub(super) fn paint_control_chrome(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    rect: Rect,
    fg: Option<Color>,
) -> Rect {
    let node = &arena[lb.node_id];
    let bg = node.css_attr.get_bg_color().or(Some(canvas.control_bg));
    let cols = canvas.columns as usize;
    let start = canvas.cell_index(rect.x, rect.y);
    for row_off in 0..rect.height {
        for col_off in 0..rect.width {
            let idx = start.saturating_add(row_off.saturating_mul(cols).saturating_add(col_off));
            if idx < canvas.layout.len() && canvas.in_region(idx) {
                let cell = &mut canvas.layout[idx];
                if cell.word.is_empty() {
                    cell.word.push(' ');
                }
                cell.bcolor = bg;
            }
        }
    }
    // CSS border opt-in frame (border thickness lives on the paint style,
    // resolved at layout time).
    let sides = &lb.paint_style.box_edges.border;
    if sides.any_visible() {
        paint_borders(lb, rect, canvas, fg, bg);
    }
    Rect::new(
        rect.x.saturating_add(sides.left.thickness()),
        rect.y.saturating_add(sides.top.thickness()),
        rect
            .width
            .saturating_sub(sides.left.thickness())
            .saturating_sub(sides.right.thickness()),
        rect
            .height
            .saturating_sub(sides.top.thickness())
            .saturating_sub(sides.bottom.thickness()),
    )
}

/// Shared caret-centred value-window computation for `<input>` fields, used by
/// both the painter and the input loop's column→character mapping.
pub(crate) use self::input::value_window;
/// Shared `<textarea>` line math (visual lines / caret position / scroll /
/// inverse mapping), used by the painter and the input loop's caret movement
/// and click placement.
pub(crate) use self::textarea::{
    textarea_caret_position, textarea_char_at, textarea_scroll_row, textarea_visual_lines,
};

/// A borrow of the canvas that the painters actually need.
///
/// `paint_layout_tree` builds a single canvas over the whole
/// [`TermLayout::layout`] buffer and reuses it across the forest. The rest
/// of the canvas API (`columns`, row math) is read-only and cheap to copy.
pub(super) struct PaintCanvas<'a> {
    pub layout: &'a mut [Block],
    pub columns: u16,
    /// Image raster cache shared with the owning [`TermLayout`], so repeated
    /// `<img>` paints across frames reuse the decoded grid instead of
    /// re-reading the file.
    pub image_cache: &'a mut ImageCache,
    /// Optional absolute canvas rect restricting cell writes. `None` paints
    /// unrestricted (full frame); `Some` is the incremental-repaint region
    /// where every write outside it is dropped so the rest of the canvas
    /// keeps the previous frame's pixels.
    pub(super) region: Option<Rect>,
    /// `<input>` edit state read by the input painter (caret / value /
    /// focus). Immutable during a paint pass.
    pub input_states: &'a HashMap<NodeId, InputRenderState>,
    /// The `<select>` node whose dropdown is currently open (`None` = none).
    /// Read by the select painter for the open-frame colour.
    pub open_select: Option<NodeId>,
    /// Default background of replaced controls when the CSS
    /// `background-color` is unset (from the session theme).
    pub(crate) control_bg: Color,
    /// Highlight colour for the selected character range (from the session
    /// theme).
    pub(crate) selection_bg: Color,
}

impl<'a> PaintCanvas<'a> {
    pub(super) fn cell_index(&self, x: usize, y: usize) -> usize {
        y.saturating_mul(self.columns as usize).saturating_add(x)
    }

    /// True when the canvas cell at 1-D index `idx` may be written by this
    /// pass. Unrestricted when no repaint region is set.
    #[inline]
    pub(super) fn in_region(&self, idx: usize) -> bool {
        match self.region {
            None => true,
            Some(r) => {
                let cols = self.columns as usize;
                let y = idx / cols;
                let x = idx - y * cols;
                x >= r.x
                    && x < r.x.saturating_add(r.width)
                    && y >= r.y
                    && y < r.y.saturating_add(r.height)
            }
        }
    }
}

/// Map `(x, y)` to a flat canvas index. Shared by the control painters
/// (`input` / `button` / `select` / `textarea`).
pub(super) fn cell_index(canvas: &PaintCanvas<'_>, x: usize, y: usize) -> usize {
    canvas.cell_index(x, y)
}

/// Write one canvas cell — word + fg + bg + plain style — gated to the
/// repaint region. Shared by the control painters.
pub(super) fn put(
    canvas: &mut PaintCanvas<'_>,
    idx: usize,
    word: Option<char>,
    fg: Option<Color>,
    bg: Option<Color>,
) {
    if idx >= canvas.layout.len() || !canvas.in_region(idx) {
        return;
    }
    let cell = &mut canvas.layout[idx];
    cell.word.clear();
    if let Some(c) = word {
        cell.word.push(c);
    }
    cell.fcolor = fg;
    cell.bcolor = bg;
    cell.style = FontStyle::empty();
}

/// Paint every box in `boxes` into `term_layout`.
///
/// Painting is deliberately serial. The top-level forest is a single box
/// for typical documents (root → `<html>`), so task-per-box parallelism
/// would spawn exactly one task and only pay `rayon` dispatch overhead;
/// the workload itself is memory-bound cell writes on a buffer of at most
/// the content height × `columns` cells. Top-level boxes occupy
/// disjoint `rect`s, so a sequential pass writes each cell at most once.
/// Children are painted inside each box — the parent must paint its
/// background before the child reads `effective_bg` for its own cells, so
/// the parent/child chain is inherently sequential anyway.
///
/// Earlier versions tried to parallelise: the original used raw-pointer
/// slicing of the whole buffer (`std::slice::from_raw_parts_mut` plus a
/// lifetime `transmute_copy`), which the project forbids as `unsafe`; a
/// later safe variant fanned out per-top-level-box bands via `split_at_mut`,
/// which measured no speedup for the single-box documents that occur in
/// practice.
pub fn paint_layout_tree(boxes: &[LayoutBox], arena: &NodeArena, term_layout: &mut TermLayout) {
    let mut max_y = 0usize;
    for b in boxes {
        let bottom = b.rect.y.saturating_add(b.rect.height.max(1));
        if bottom > max_y {
            max_y = bottom;
        }
    }
    if max_y as u32 > term_layout.rendered_rows {
        term_layout.rendered_rows = max_y as u32;
    }

    let columns = term_layout.columns;
    let mut canvas = PaintCanvas {
        layout: &mut term_layout.layout,
        columns,
        image_cache: &mut term_layout.image_cache,
        region: None,
        input_states: &term_layout.input_states,
        open_select: term_layout.open_select,
        control_bg: term_layout.theme.control_bg,
        selection_bg: term_layout.theme.selection_bg,
    };
    for b in boxes {
        paint_box(b, arena, &mut canvas, None, None);
    }
    fill_canvas_background(&mut canvas, boxes, arena, max_y);
}

/// Repaint only the cells inside `region`, leaving the rest of the canvas
/// untouched.
///
/// Backs the incremental render path: the layout has already been re-run and
/// every box rect verified unchanged against the cached layout, so only the
/// cells the dirty nodes' boxes (and their ancestors' background fills) cover
/// can differ from the previous frame. Boxes whose rect does not intersect
/// `region` are skipped entirely; the rest paint normally, with every cell
/// write gated to the region by [`PaintCanvas::in_region`]. Because only the
/// region's cells are rewritten, the diff renderer emits exactly the cells
/// that changed.
pub fn paint_layout_tree_region(
    boxes: &[LayoutBox],
    arena: &NodeArena,
    term_layout: &mut TermLayout,
    region: Rect,
) {
    let mut max_y = 0usize;
    for b in boxes {
        let bottom = b.rect.y.saturating_add(b.rect.height.max(1));
        if bottom > max_y {
            max_y = bottom;
        }
    }
    if max_y as u32 > term_layout.rendered_rows {
        term_layout.rendered_rows = max_y as u32;
    }

    let columns = term_layout.columns;
    let mut canvas = PaintCanvas {
        layout: &mut term_layout.layout,
        columns,
        image_cache: &mut term_layout.image_cache,
        region: Some(region),
        input_states: &term_layout.input_states,
        open_select: term_layout.open_select,
        control_bg: term_layout.theme.control_bg,
        selection_bg: term_layout.theme.selection_bg,
    };
    for b in boxes {
        if intersect_rects(b.rect, region).is_some() {
            paint_box(b, arena, &mut canvas, None, None);
        }
    }
    fill_canvas_background(&mut canvas, boxes, arena, max_y);
}

/// Paint the canvas background below the content, browser-style.
///
/// In browsers the `<html>` element's `background-color` paints the whole
/// canvas; when `<html>` is transparent the `<body>` background propagates
/// instead. Boxes only fill their own rects, so a short page would otherwise
/// leave the terminal rows below the content unpainted. This fills every
/// still-default cell from the content bottom down to the canvas end with the
/// propagated colour. Cell writes stay gated by [`PaintCanvas::in_region`], so
/// the incremental-repaint path never touches cells outside its region.
fn fill_canvas_background(
    canvas: &mut PaintCanvas<'_>,
    boxes: &[LayoutBox],
    arena: &NodeArena,
    content_bottom: usize,
) {
    let Some(bg) = find_canvas_background(boxes, arena) else {
        return;
    };
    let cols = canvas.columns as usize;
    let rows = canvas.layout.len() / cols;
    let start_row = content_bottom.min(rows);
    let region = canvas.region;
    for y in start_row..rows {
        let row_start = y * cols;
        for (i, cell) in canvas.layout[row_start..row_start + cols]
            .iter_mut()
            .enumerate()
        {
            let in_region = match region {
                None => true,
                Some(r) => {
                    i >= r.x
                        && i < r.x.saturating_add(r.width)
                        && y >= r.y
                        && y < r.y.saturating_add(r.height)
                }
            };
            if !in_region {
                continue;
            }
            // Only still-default cells: never clobber content that happens to
            // sit below `content_bottom` (e.g. a positioned child).
            if cell.word == " " && cell.bcolor.is_none() && cell.style.is_empty() {
                cell.bcolor = Some(bg);
            }
        }
    }
}

/// The colour the canvas should paint: the `<html>` background, or the
/// `<body>` background when `<html>` has none (browser propagation). `None`
/// when neither element paints one.
fn find_canvas_background(boxes: &[LayoutBox], arena: &NodeArena) -> Option<Color> {
    for b in boxes {
        if let Some(bg) = box_canvas_background(b, arena) {
            return Some(bg);
        }
    }
    None
}

fn box_canvas_background(lb: &LayoutBox, arena: &NodeArena) -> Option<Color> {
    let node = &arena[lb.node_id];
    if node.node_type == NodeType::Html {
        if let Some(bg) = node.css_attr.get_bg_color() {
            return Some(bg);
        }
        for child in &lb.children {
            if arena[child.node_id].node_type == NodeType::Body
                && let Some(bg) = arena[child.node_id].css_attr.get_bg_color()
            {
                return Some(bg);
            }
        }
    }
    // Fragment documents may lack an `<html>` wrapper — recurse.
    for child in &lb.children {
        if let Some(bg) = box_canvas_background(child, arena) {
            return Some(bg);
        }
    }
    None
}

/// Clamp `a` to the intersection with `b`, or return `None` when the two
/// rectangles do not overlap. Both rects are in absolute canvas coordinates.
pub(super) fn intersect_rects(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.x.saturating_add(a.width).min(b.x.saturating_add(b.width));
    let y1 =
        a.y.saturating_add(a.height)
            .min(b.y.saturating_add(b.height));
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// Clamp a box rect to the canvas width so its frame / text writes never
/// land past the terminal's right edge, where `cell_index` would wrap into
/// the next row of the flat canvas buffer.
///
/// [`paint_block`] already clamps the rect it paints with; the replaced
/// element painters (`<input>` / `<textarea>` / `<button>` / `<select>`) get
/// the same treatment here, since a flex-placed item can hand them a rect
/// wider than the viewport — without the clamp the frame breaks (missing
/// right side) and the wrapped writes corrupt the rows below.
pub(super) fn clamp_rect_to_canvas(r: Rect, canvas_cols: usize) -> Option<Rect> {
    if r.x >= canvas_cols || r.width == 0 || r.height == 0 {
        return None;
    }
    Some(Rect::new(
        r.x,
        r.y,
        r.width.min(canvas_cols - r.x),
        r.height,
    ))
}

/// Paint one box (and its subtree) into the canvas.
///
/// `clip` is the box's inherited clip region (absolute canvas coordinates):
/// the intersection of every ancestor's border-box. Content that would be
/// painted outside the clip is dropped instead of spilling onto rows below
/// or beyond the box — on very small terminals the flexible algorithm can
/// hand a box a rect smaller than its content, and without this clip the
/// wrapped lines / block children would paint over the next sibling.
fn paint_box(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    parent_bg: Option<Color>,
    clip: Option<Rect>,
) {
    let canvas_cols = canvas.columns as usize;
    // A box laid out wider than the terminal (e.g. a flex row whose items
    // overflow the viewport) must not paint past the right edge, where the
    // flat canvas wraps writes into the next row. Clamp the rect once here so
    // the content area / children clip derived below also end at the canvas
    // edge — otherwise a child's background would cover the parent's
    // (clamped) border cell and leave the frame with a hole on the right
    // side when the terminal shrinks.
    let rect = if lb.rect.x >= canvas_cols || lb.rect.width == 0 {
        lb.rect
    } else {
        Rect::new(
            lb.rect.x,
            lb.rect.y,
            lb.rect.width.min(canvas_cols.saturating_sub(lb.rect.x)),
            lb.rect.height,
        )
    };
    // This box's own clip = inherited clip ∩ own border-box. A box never
    // paints outside its ancestors' borders.
    let own_clip = match clip {
        Some(c) => match intersect_rects(c, rect) {
            Some(r) => r,
            None => return,
        },
        None => rect,
    };
    if own_clip.width == 0 || own_clip.height == 0 {
        // Zero-area box: no chrome of its own, but positioned children are
        // anchored to a containing block (or offset out of this area) and
        // must still paint. Static / relative-in-place children stay clipped
        // away by the zero clip.
        for child in sorted_children(lb) {
            if !matches!(child.positioning.position, Position::Static) {
                paint_box(child, arena, canvas, parent_bg, None);
            }
        }
        return;
    }
    let my_bg = match &lb.kind {
        BoxKind::Normal => arena[lb.node_id].css_attr.get_bg_color(),
        BoxKind::Replaced(_) => None,
        BoxKind::Anonymous => None,
    };
    // `visibility: hidden` (inherited, so a child's `visible` re-shows it):
    // the box keeps its layout space but paints nothing of its own chrome /
    // inline content. Children still descend — each node checks its own
    // resolved visibility, exactly like the browser.
    let visibility_hidden =
        matches!(arena[lb.node_id].css_attr.get_visibility(), Visibility::Hidden);
    if !visibility_hidden {
        match &lb.kind {
            BoxKind::Normal => {
                paint_block(lb, arena, canvas, my_bg, parent_bg, own_clip);
            }
            BoxKind::Replaced(ReplacedContent::Image { cols, rows, .. }) => {
                paint_image(lb, canvas, *cols, *rows, own_clip);
            }
            BoxKind::Replaced(ReplacedContent::Input) => {
                paint_input(lb, arena, canvas, own_clip);
            }
            BoxKind::Replaced(ReplacedContent::Select) => {
                paint_select(lb, arena, canvas, own_clip);
            }
            BoxKind::Replaced(ReplacedContent::Textarea) => {
                paint_textarea(lb, arena, canvas, own_clip);
            }
            BoxKind::Replaced(ReplacedContent::Button) => {
                paint_button(lb, arena, canvas, own_clip);
            }
            BoxKind::Anonymous => {
                // Anonymous boxes have no chrome of their own; their visible
                // background is whatever their children paint, so just descend.
            }
        }
    }
    // The effective bg visible at this box's rect becomes the parent bg for
    // any descendant. If the current box has its own bg, that wins; otherwise
    // we inherit. This is what lets `<aside bg=cyan><p>30col</p></aside>`
    // render the text with cyan bg even though `<p>` itself has no bg.
    let effective_bg = my_bg.or(parent_bg);
    // Children are positioned inside the box's content-box (the rect shrunk
    // by border + padding); anything that spills out of it — e.g. a squeezed
    // flex item's wrapped lines — is dropped rather than painted over the
    // padding / border rows of this box or the sibling below. The content
    // area is NOT rounded up to 1 cell: when a box is smaller than its
    // border + padding the true content area is empty and the children are
    // dropped entirely, so no descendant can ever overwrite a frame cell.
    let edges = &lb.paint_style.box_edges;
    let content_area = Rect::new(
        rect.x
            .saturating_add(edges.border.left.thickness())
            .saturating_add(edges.padding.left),
        rect.y
            .saturating_add(edges.border.top.thickness())
            .saturating_add(edges.padding.top),
        rect.width.saturating_sub(edges.inner_horizontal()),
        rect.height.saturating_sub(edges.inner_vertical()),
    );
    let children_clip = intersect_rects(own_clip, content_area);
    paint_children(lb, arena, canvas, effective_bg, children_clip);
}

/// Paint `lb`'s children in stacking order.
///
/// `children_clip` is inherited by static / relative children (the parent's
/// content area). Detached positioned children are not clipped by the parent
/// — they are anchored to a containing block that may extend beyond it — and
/// their own rects clip their descendants via [`paint_box`].
///
/// When the parent's content area is empty (`children_clip` is `None` — the
/// box is smaller than its border + padding), static children are dropped via
/// a zero-area clip so they can never paint over the parent's frame, while
/// positioned children keep painting unclipped.
fn paint_children(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    effective_bg: Option<Color>,
    children_clip: Option<Rect>,
) {
    for child in sorted_children(lb) {
        // Positioned children (absolute / fixed / relative) may paint outside
        // the parent's content area — they are anchored to a containing block
        // or offset out of flow — so they are not clipped by the ancestor
        // chain. Their own rects still clip their descendants via
        // [`paint_box`]. Static children inherit the parent's clip.
        let clip = match child.positioning.position {
            Position::Static => children_clip.or(Some(Rect::new(0, 0, 0, 0))),
            _ => None,
        };
        paint_box(child, arena, canvas, effective_bg, clip);
    }
}

/// Children in paint stacking order.
///
/// Higher `z-index` paints later (on top); among equal values a positioned
/// box paints above static content (browser rule); the stable sort keeps DOM
/// order within the same level.
///
/// The common case — no positioned child and uniform `z-index` — leaves the
/// sort key identical for every child, so a stable sort cannot reorder
/// anything. [`SortedChildren`] fast-paths that case by iterating the
/// children slice directly (zero allocation); only when the keys actually
/// differ does it collect into a `Vec` and sort.
fn sorted_children(lb: &LayoutBox) -> SortedChildren<'_> {
    let key = |c: &LayoutBox| {
        (
            c.positioning.z_index.unwrap_or(0),
            !matches!(c.positioning.position, Position::Static) as u8,
        )
    };
    let first = lb.children.first().map(|c| key(c));
    let needs_sort = lb.children.iter().any(|c| Some(key(c)) != first);
    if needs_sort {
        let mut children: Vec<&LayoutBox> = lb.children.iter().collect();
        children.sort_by_key(|c| key(c));
        SortedChildren::Sorted(children.into_iter())
    } else {
        SortedChildren::Straight(lb.children.iter())
    }
}

/// Iterator over a box's children in paint stacking order — see
/// [`sorted_children`].
enum SortedChildren<'a> {
    /// Sort keys are all equal: iterate the slice as-is (no allocation).
    Straight(std::slice::Iter<'a, LayoutBox>),
    /// Mixed sort keys: a collected, sorted `Vec`.
    Sorted(std::vec::IntoIter<&'a LayoutBox>),
}

impl<'a> Iterator for SortedChildren<'a> {
    type Item = &'a LayoutBox;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            SortedChildren::Straight(it) => it.next(),
            SortedChildren::Sorted(it) => it.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            SortedChildren::Straight(it) => it.size_hint(),
            SortedChildren::Sorted(it) => it.size_hint(),
        }
    }
}

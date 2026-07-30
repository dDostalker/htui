//! Outer-geometry helpers for block layout.
//!
//! These helpers take the box's resolved CSS edges and the cursor row, and
//! return the border-box rectangle plus the inner content-box to pass to
//! nested children.

use crate::{
    html::box_edges::BoxEdges,
    layout::{BoxConstraint, Cursor, Rect},
};

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

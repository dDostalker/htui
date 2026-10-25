//! Single-cell layout (`<td>` / `<th>`) — invoked by [`super::table::layout_table`]
//! once the column width has been computed.

use crate::{
    graphics::{FontStyle, block_style},
    html::box_edges::BoxEdges,
    layout::{
        BoxConstraint, Cursor, InlineRun, LayoutBox, LayoutContext, PaintStyle, Rect,
        inner_formatting_context::flow::layout_children,
        layoutbox::{collect_runs, wrap_runs},
    },
    tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells},
};

/// Information about a single cell inside a row.
#[derive(Clone, Copy)]
pub struct CellInfo {
    pub node_id: NodeId,
    /// `true` for `<th>` cells; the font-style / alignment defaults are
    /// surfaced for callers that want to apply them.
    pub is_header: bool,
}

/// Measure one cell at its column width: returns the row height the cell
/// needs plus its wrapped inline runs. Called once per cell by
/// `layout_table`'s row-height pass; `layout_cell` reuses the runs, so the
/// collect + wrap pipeline runs exactly once per cell per layout pass
/// (identical inputs — same arena, same column width — would produce the
/// same lines a second time).
///
/// The height is the taller of the wrapped inline-text height (each visual
/// line takes `font_size_cells` rows) and the laid-out height of the cell's
/// block children, plus the cell's own vertical border / padding. Row heights
/// drive the table geometry; without this, a cell whose text wraps (common
/// when the terminal is narrow enough that the column shrinks) would paint
/// its wrapped lines over the table's bottom border, and a
/// `<td><div>…</div></td>` whose block child stacks several rows would paint
/// the child over the next table row.
///
/// `is_header` applies the `<th>` UA bold default to the runs — bold glyphs
/// occupy the same widths (the height is unaffected), but the reused runs
/// must carry it for the paint phase.
pub fn measure_cell(
    arena: &NodeArena,
    cell_id: NodeId,
    col_width: usize,
    is_header: bool,
    context: LayoutContext,
) -> (usize, Vec<Vec<InlineRun>>) {
    let node = &arena[cell_id];
    let own_font_size_cells =
        effective_font_size_cells(arena, cell_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);
    let cell_edges: BoxEdges =
        node.css_attr
            .get_box_edges(col_width, child_context.font, child_context.viewport);
    let content_width = col_width
        .saturating_sub(cell_edges.inner_horizontal())
        .max(1);

    let block_fg = node.css_attr.get_color();
    let mut block_font_style = block_style(&node.css_attr);
    if is_header {
        for &(prop, value) in node.node_type.inline_ua_declarations() {
            if prop == "font-weight" && !node.css_attr.is_set(prop) && value == "bold" {
                block_font_style |= FontStyle::BOLD;
            }
        }
    }
    // The block's own background is resolved from the fresh cascade at paint
    // time (`effective_bg`), not baked into the runs (see the block layout
    // counterpart — a `:hover` re-cascade must not reuse stale run colours).
    let body_runs = collect_runs(arena, cell_id, node, block_fg, None, block_font_style);
    let wrapped_lines = if body_runs.is_empty() {
        Vec::new()
    } else {
        wrap_runs(
            &body_runs,
            content_width,
            own_font_size_cells,
            node.css_attr.get_white_space(&node.node_type),
            node.css_attr.get_word_break(),
            node.css_attr.get_overflow_wrap(),
        )
    };
    // Whitespace-only runs collapse away (wrap_runs returns no visual lines),
    // but the cell still reserves one row.
    let inline_rows = wrapped_lines
        .len()
        .max(1)
        .saturating_mul(own_font_size_cells.max(1));

    // Block children claim their own rows inside the cell (laid out via
    // `layout_children`); their real laid-out height must feed the row
    // height. Measure them with a scratch cursor against the same content
    // width the real cell layout uses. Detached (absolute / fixed) children
    // claim no flow space and are excluded, exactly like the block layout.
    let block_extent = {
        let mut scratch = Cursor {
            y: 0,
            prev_bottom_margin: 0,
        };
        let children = layout_children(
            arena,
            &node.children,
            BoxConstraint {
                x: 0,
                width: content_width,
            },
            &mut scratch,
            child_context,
        );
        children
            .iter()
            .filter(|b| {
                !matches!(
                    b.positioning.position,
                    crate::html::css_parse::Position::Absolute
                        | crate::html::css_parse::Position::Fixed
                )
            })
            .map(|b| b.rect.y.saturating_add(b.rect.height))
            .max()
            .unwrap_or(0)
    };

    let height = inline_rows
        .max(block_extent)
        .saturating_add(cell_edges.border.top.thickness())
        .saturating_add(cell_edges.padding.top)
        .saturating_add(cell_edges.border.bottom.thickness())
        .saturating_add(cell_edges.padding.bottom)
        .max(1);
    (height, wrapped_lines)
}

/// Build a single cell's [`LayoutBox`] at the given `rect`.
/// Build a single cell's [`LayoutBox`] at the given `rect`.
///
/// `wrapped_lines` comes from [`measure_cell`] (same cell, same column
/// width, same layout pass) — re-collecting and re-wrapping here would
/// produce identical lines a second time.
pub fn layout_cell(
    arena: &NodeArena,
    cell_id: NodeId,
    rect: Rect,
    is_header: bool,
    context: LayoutContext,
    wrapped_lines: Vec<Vec<InlineRun>>,
    hide_if_empty: bool,
) -> LayoutBox {
    let node = &arena[cell_id];

    let own_font_size_cells =
        effective_font_size_cells(arena, cell_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);

    // `empty-cells: hide` — a cell with no visible content (no inline runs
    // and no element children) paints no chrome: borders, padding and
    // background all drop, exactly like the CSS default-tables behaviour.
    let cell_is_empty = wrapped_lines.iter().all(|l| l.is_empty())
        && node.children.iter().all(|&c| {
            matches!(arena.get(c).map(|n| n.node_type), Some(NodeType::Text))
                && arena
                    .get(c)
                    .and_then(|n| n.data.as_deref())
                    .is_none_or(str::is_empty)
        });
    let suppress_chrome = hide_if_empty && cell_is_empty;

    let cell_edges: BoxEdges = if suppress_chrome {
        BoxEdges::default()
    } else {
        node.css_attr
            .get_box_edges(rect.width, child_context.font, child_context.viewport)
    };

    let block_bg = if suppress_chrome {
        None
    } else {
        node.css_attr.get_bg_color()
    };
    // Browser UA stylesheet: `th { font-weight: bold; text-align: center }` —
    // the bold half already rode in on the reused runs (see [`measure_cell`]);
    // the alignment half resolves here for the paint phase, each pair
    // yielding to any author declaration / inherited value
    // (`th { text-align: left }` wins, same cascade the other UA defaults
    // follow).
    let mut text_align = node.css_attr.get_text_align();
    if is_header {
        for &(prop, value) in node.node_type.inline_ua_declarations() {
            if prop == "text-align" && !node.css_attr.is_set(prop) && value == "center" {
                text_align = crate::html::css_parse::TextAlign::Center;
            }
        }
    }

    let fill_bg = block_bg.is_some();

    let inner_content_x = rect.x + cell_edges.border.left.thickness() + cell_edges.padding.left;
    let inner_content_y = rect.y + cell_edges.border.top.thickness() + cell_edges.padding.top;
    let inner_content_width = rect
        .width
        .saturating_sub(cell_edges.inner_horizontal())
        .max(1);
    let mut child_cursor = Cursor {
        y: inner_content_y,
        prev_bottom_margin: 0,
    };
    let child_containing = BoxConstraint {
        x: inner_content_x,
        width: inner_content_width,
    };
    let children = layout_children(
        arena,
        &node.children,
        child_containing,
        &mut child_cursor,
        child_context,
    );

    let paint_style = PaintStyle {
        box_edges: cell_edges,
        runs: if wrapped_lines.is_empty() || wrapped_lines.iter().all(|l| l.is_empty()) {
            None
        } else {
            Some(wrapped_lines)
        },
        column_layout: None,
        font_size_cells: own_font_size_cells,
        line_height_cells: own_font_size_cells.max(1),
        border_radius: crate::html::css_parse::CornerRadii::default(),
        fill_bg,
        spacing_after: 0,
        text_align,
    };

    LayoutBox::new_normal(
        cell_id,
        rect,
        paint_style,
        node.css_attr.outer_display(&node.node_type),
        children,
    )
}

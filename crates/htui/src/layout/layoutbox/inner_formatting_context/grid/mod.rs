//! CSS Grid formatting context.
//!
//! Implements the engine's terminal-grid subset:
//!
//! - display: grid / inline-grid
//! - track lists with auto, lengths/percentages, fr, minmax()/fit-content(),
//!   and repeat() (integer counts parse eagerly; auto-fill/auto-fit resolve
//!   against the definite container size, auto-fit collapsing its empty
//!   tail tracks)
//! - explicit line-number/span placement and automatic row/column placement
//!   (including dense)
//! - `grid-template-areas` named areas; a bare area name in a placement
//!   resolves to that area's edge line, unknown names fall back to `auto`
//! - implicit track lists cycled across consecutive implicit tracks
//! - row/column gaps and per-item start/end/center/stretch alignment
//!   (align-self/justify-self over align-items/justify-items defaults)
//!
//! Named lines (beyond area names) and subgrid are not part of this
//! implementation; unsupported declarations are rejected at the CSS value
//! layer rather than guessed.

use std::collections::HashSet;
use std::sync::Arc;

use crate::html::css_parse::{
    AlignContent, AlignItems, GridAreas, GridAutoFlow, GridLine, GridPlacements, GridTrack,
    JustifyContent, OverflowWrap, WhiteSpace, WordBreak,
};
use crate::layout::{
    BoxConstraint, Cursor, InlineRun, LayoutBox, LayoutContext, OuterDisplay, PaintStyle, Rect,
    inner_formatting_context::flow::{apply_horizontal_auto_margins, constrained_block_width},
    layoutbox::{
        flatten_contents_children, generation::GeneratedBoxKind, text_node_run, visible_children,
        wrap_runs,
    },
};
use crate::tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells};

use super::flow::dispatch as flow_dispatch;
use unicode_segmentation::UnicodeSegmentation;

/// A grid item before track sizing and final geometry.
struct GridItemPrep {
    source: GridItemSource,
    natural_width: usize,
    natural_height: usize,
    row: RawPlacement,
    column: RawPlacement,
    align: Option<AlignItems>,
    justify: Option<AlignItems>,
    order: i32,
}

enum GridItemSource {
    Node(NodeId),
    Text(AnonymousGridText),
}

struct AnonymousGridText {
    runs: Vec<InlineRun>,
    font_size_cells: usize,
    line_height_cells: usize,
    white_space: WhiteSpace,
    word_break: WordBreak,
    overflow_wrap: OverflowWrap,
}

#[derive(Clone)]
struct RawPlacement {
    start: Option<GridLine>,
    end: Option<GridLine>,
}

impl RawPlacement {
    fn auto() -> Self {
        Self {
            start: None,
            end: None,
        }
    }

    fn span(&self) -> usize {
        match (&self.start, &self.end) {
            (Some(GridLine::Span(n)), _) | (_, Some(GridLine::Span(n))) => (*n).max(1),
            _ => 1,
        }
    }
}

#[derive(Clone, Copy)]
struct Placed {
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
}

#[derive(Clone, Copy)]
struct TrackArea {
    origin: usize,
    size: usize,
}

/// Entry point used by the unified box-geometry dispatcher.
pub(crate) fn layout_grid_dispatch(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let generated = crate::layout::layoutbox::generation::generate_node(arena, node_id)?;
    if generated.inner_formatting_context != crate::layout::FormattingContext::Grid {
        return None;
    }
    Some(layout_grid_container(
        arena,
        node_id,
        containing,
        cursor,
        context,
        pre_allocated_rect,
    ))
}

fn layout_grid_container(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> LayoutBox {
    let node = &arena[node_id];
    let css = &node.css_attr;
    let own_font_size_cells =
        effective_font_size_cells(arena, node_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);

    let edges = css.get_box_edges(containing.width, child_context.font, child_context.viewport);
    let line_height_cells = crate::html::css_parse::resolve_line_height_cells(
        css.get_line_height(),
        own_font_size_cells,
        child_context.font,
        child_context.viewport,
    );

    let (outer_x, outer_y_top, block_width, preallocated_height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.y, rect.width.max(1), Some(rect.height.max(1))),
        None => {
            cursor.collapse_and_skip(edges.margin.top);
            let outer_y_top = cursor.y;
            let available =
                (containing.width as i64 - edges.margin.horizontal() as i64).max(0) as usize;
            let block_width = constrained_block_width(
                css,
                available,
                edges,
                child_context.font,
                child_context.viewport,
            );
            let mut edges = edges;
            let margin_auto = css.get_margin_auto();
            apply_horizontal_auto_margins(
                (margin_auto.left, margin_auto.right),
                &mut edges,
                available,
                block_width,
            );
            (
                (containing.x as i64 + edges.margin.left.max(0) as usize as i64).max(0) as usize,
                outer_y_top,
                block_width,
                None,
            )
        }
    };

    let content_x = outer_x + edges.border.left.thickness() + edges.padding.left;
    let content_width = block_width.saturating_sub(edges.inner_horizontal()).max(1);
    let content_y = outer_y_top + edges.border.top.thickness() + edges.padding.top;
    let content_constraint = BoxConstraint {
        x: content_x,
        width: content_width,
    };

    let container_css_height = preallocated_height
        .map(|height| height.saturating_sub(edges.inner_vertical()).max(1))
        .or_else(|| css.get_height(None, child_context.font, child_context.viewport));
    let definite_content_height = container_css_height;

    let auto_columns = css.get_grid_auto_columns();
    let auto_rows = css.get_grid_auto_rows();
    let auto_flow = css.get_grid_auto_flow();
    let gap = css.get_gap(child_context.font, child_context.viewport);
    let template = css.get_grid_template(child_context.font, child_context.viewport);
    // `repeat(auto-fill | auto-fit, …)` expands against the definite sizes
    // before placement; `fit` segments remember their span for collapsing.
    let (mut template_columns, column_repeats) = expand_auto_repeats(
        &template.columns,
        Some(content_width),
        gap.column,
        child_context.font,
        child_context.viewport,
    );
    let (mut template_rows, row_repeats) = expand_auto_repeats(
        &template.rows,
        definite_content_height,
        gap.row,
        child_context.font,
        child_context.viewport,
    );
    let areas = css.get_grid_template_areas();
    let align_items = css.get_align_items();
    let justify_items = css.get_justify_items();
    let align_content = css.get_align_content();
    let justify_content = css.get_justify_content();

    let mut items = collect_grid_items(
        arena,
        node_id,
        own_font_size_cells,
        line_height_cells,
        child_context,
    );
    items.sort_by_key(|item| item.order);

    let mut placements: Vec<Placed> = Vec::with_capacity(items.len());
    let mut occupied: HashSet<(usize, usize)> = HashSet::new();
    // `grid-template-areas` extends the explicit grid: tracks beyond
    // `grid-template-columns`/`rows` stay implicit and use `grid-auto-*`.
    let mut column_count = explicit_count(&template_columns)
        .max(areas.as_ref().map(|a| a.column_count()).unwrap_or(0))
        .max(min_tracks(&items, Axis::Column))
        .max(1);
    let mut row_count = explicit_count(&template_rows)
        .max(areas.as_ref().map(|a| a.row_count()).unwrap_or(0))
        .max(min_tracks(&items, Axis::Row))
        .max(1);
    let mut flow_cursor = (0usize, 0usize);

    for item in &items {
        let placed = place_item(
            item,
            auto_flow,
            column_count,
            row_count,
            &mut flow_cursor,
            &occupied,
        );
        mark_occupied(&mut occupied, placed);
        placements.push(placed);
    }

    // `auto-fit`: trailing tracks of a tail auto-repeat that no item spans
    // collapse away (an interior segment would shift later tracks, so it
    // behaves like `auto-fill`).
    collapse_fit_tail(
        &mut template_columns,
        &column_repeats,
        &placements,
        Axis::Column,
    );
    collapse_fit_tail(&mut template_rows, &row_repeats, &placements, Axis::Row);

    // Recompute the grid extents from the (possibly collapsed) template so
    // collapsed tracks stop reserving space.
    column_count = explicit_count(&template_columns)
        .max(areas.as_ref().map(|a| a.column_count()).unwrap_or(0))
        .max(1);
    row_count = explicit_count(&template_rows)
        .max(areas.as_ref().map(|a| a.row_count()).unwrap_or(0))
        .max(1);
    for placed in &placements {
        column_count = column_count.max(placed.column.saturating_add(placed.column_span));
        row_count = row_count.max(placed.row.saturating_add(placed.row_span));
    }

    let column_hints = track_hints(
        items.iter().zip(&placements),
        |item| item.natural_width,
        |placed| (placed.column, placed.column_span),
        column_count,
    );
    let row_hints = track_hints(
        items.iter().zip(&placements),
        |item| item.natural_height,
        |placed| (placed.row, placed.row_span),
        row_count,
    );

    let columns = track_sizes(
        &template_columns,
        &auto_columns,
        &column_hints,
        Some(content_width),
        gap.column,
        child_context.font,
        child_context.viewport,
    );
    let rows = track_sizes(
        &template_rows,
        &auto_rows,
        &row_hints,
        definite_content_height,
        gap.row,
        child_context.font,
        child_context.viewport,
    );

    let column_areas = track_areas(&columns, content_x, gap.column);
    let row_areas = track_areas(&rows, content_y, gap.row);
    let grid_width: usize = columns
        .iter()
        .copied()
        .fold(0usize, usize::saturating_add)
        .saturating_add(gap.column.saturating_mul(columns.len().saturating_sub(1)));
    let grid_height: usize = rows
        .iter()
        .copied()
        .fold(0usize, usize::saturating_add)
        .saturating_add(gap.row.saturating_mul(rows.len().saturating_sub(1)));
    let x_shift = track_alignment_offset(justify_content, content_width, grid_width);
    let y_shift = track_alignment_offset(
        align_content_to_justify(align_content),
        definite_content_height.unwrap_or(grid_height),
        grid_height,
    );

    let mut positioned_boxes = Vec::new();
    for item in &items {
        let child = match &item.source {
            GridItemSource::Node(child_id) => Some(*child_id),
            GridItemSource::Text(_) => None,
        };
        let Some(child_id) = child else { continue };
        let positioning = arena[child_id].css_attr.get_positioning(
            content_width,
            child_context.font,
            child_context.viewport,
        );
        if matches!(
            positioning.position,
            crate::html::css_parse::Position::Absolute | crate::html::css_parse::Position::Fixed
        ) {
            let mut local_cursor = Cursor {
                y: 0,
                prev_bottom_margin: 0,
            };
            if let Some(b) = flow_dispatch(
                arena,
                child_id,
                content_constraint,
                &mut local_cursor,
                child_context,
                None,
            ) {
                positioned_boxes.push(b);
            }
        }
    }

    let mut child_boxes = positioned_boxes;
    for (item, placed) in items.iter().zip(&placements) {
        let column_area =
            spanned_area(&column_areas, placed.column, placed.column_span).unwrap_or(TrackArea {
                origin: content_x,
                size: content_width.max(1),
            });
        let row_area = spanned_area(&row_areas, placed.row, placed.row_span).unwrap_or(TrackArea {
            origin: content_y,
            size: 1,
        });
        let column_area = TrackArea {
            origin: column_area.origin.saturating_add(x_shift),
            size: column_area.size,
        };
        let row_area = TrackArea {
            origin: row_area.origin.saturating_add(y_shift),
            size: row_area.size,
        };

        let Some(child_box) = layout_grid_item(
            arena,
            item,
            column_area,
            row_area,
            align_items,
            justify_items,
            definite_content_height,
            content_width,
            child_context,
            node_id,
        ) else {
            continue;
        };
        child_boxes.push(child_box);
    }

    let track_height = grid_height.max(1);
    let mut outer_height = match preallocated_height {
        Some(height) => {
            let inner_v = edges.inner_vertical();
            let owns_height = css.scrolls_vertically()
                || css.get_overflow() == crate::html::css_parse::Overflow::Clip
                || css
                    .get_height(Some(height), child_context.font, child_context.viewport)
                    .is_some();
            if owns_height {
                height
            } else {
                height.max(track_height.saturating_add(inner_v))
            }
        }
        None => container_css_height
            .map(|height| height.max(track_height))
            .unwrap_or(track_height),
    };
    if preallocated_height.is_none() {
        let inner_v = edges.inner_vertical();
        let sizing = css.get_box_sizing();
        let to_content = |h: usize| match sizing {
            crate::html::css_parse::BoxSizing::BorderBox => h.saturating_sub(inner_v),
            crate::html::css_parse::BoxSizing::ContentBox => h,
        };
        if let Some(m) = css
            .get_min_height(None, child_context.font, child_context.viewport)
            .map(to_content)
        {
            outer_height = outer_height.max(m);
        }
        if let Some(m) = css
            .get_max_height(None, child_context.font, child_context.viewport)
            .map(to_content)
        {
            outer_height = outer_height.min(m).max(1);
        }
    }

    if preallocated_height.is_none() {
        let total_box_height = outer_height
            .saturating_add(edges.border.top.thickness())
            .saturating_add(edges.padding.top)
            .saturating_add(edges.border.bottom.thickness())
            .saturating_add(edges.padding.bottom);
        cursor.y = outer_y_top
            .saturating_add(total_box_height)
            .saturating_add(edges.margin.bottom);
        cursor.leave_bottom_margin(edges.margin.bottom);
    }

    let total_box_height = if preallocated_height.is_some() {
        outer_height
    } else {
        outer_height
            .saturating_add(edges.border.top.thickness())
            .saturating_add(edges.padding.top)
            .saturating_add(edges.border.bottom.thickness())
            .saturating_add(edges.padding.bottom)
    };
    let rect = Rect::new(outer_x, outer_y_top, block_width, total_box_height);
    let paint_style = PaintStyle {
        box_edges: edges,
        runs: None,
        column_layout: None,
        font_size_cells: own_font_size_cells,
        line_height_cells,
        border_radius: css.get_border_radius(
            content_width,
            child_context.font,
            child_context.viewport,
        ),
        fill_bg: css.get_bg_color().is_some(),
        spacing_after: 0,
        text_align: css.get_text_align(),
        text_indent: 0,
        scrollbar: None,
    };

    LayoutBox::new_normal_with_context(
        node_id,
        rect,
        paint_style,
        css.outer_display(&node.node_type),
        crate::layout::FormattingContext::Grid,
        child_boxes,
    )
}

/// Collect element children and contiguous raw-text children as grid items.
fn collect_grid_items(
    arena: &NodeArena,
    node_id: NodeId,
    font_size_cells: usize,
    line_height_cells: usize,
    context: LayoutContext,
) -> Vec<GridItemPrep> {
    let node = &arena[node_id];
    let fg = node.css_attr.get_color();
    let bg = node.css_attr.get_bg_color();
    let font_style = crate::graphics::block_style(&node.css_attr);
    let transform = node.css_attr.get_text_transform();
    let white_space = node.css_attr.get_white_space(&node.node_type);
    let word_break = node.css_attr.get_word_break();
    let overflow_wrap = node.css_attr.get_overflow_wrap();
    // One pass over the matrix per container; per-item name lookups then
    // cost an O(area-count) scan instead of an O(cell-count) rescan.
    let areas = node
        .css_attr
        .get_grid_template_areas()
        .map(|areas| collect_area_bounds(&areas));
    let mut items = Vec::new();
    let mut text_runs = Vec::new();

    let flush_text = |items: &mut Vec<GridItemPrep>, runs: &mut Vec<InlineRun>| {
        if runs.is_empty() {
            return;
        }
        let width = runs
            .iter()
            .filter(|run| !run.force_break)
            .map(|run| {
                run.text
                    .graphemes(true)
                    .map(crate::graphics::grapheme_width)
                    .sum::<usize>()
            })
            .sum::<usize>()
            .max(1);
        items.push(GridItemPrep {
            source: GridItemSource::Text(AnonymousGridText {
                runs: std::mem::take(runs),
                font_size_cells,
                line_height_cells,
                white_space,
                word_break,
                overflow_wrap,
            }),
            natural_width: width,
            natural_height: line_height_cells.max(1),
            row: RawPlacement::auto(),
            column: RawPlacement::auto(),
            align: None,
            justify: None,
            order: 0,
        });
    };

    let dom_children = flatten_contents_children(arena, &visible_children(arena, node_id));
    for child_id in dom_children {
        let child = &arena[child_id];
        if child.node_type == NodeType::Text {
            if let Some(run) =
                text_node_run(arena, child_id, fg, bg, font_style, transform, white_space)
            {
                text_runs.push(run);
            }
            continue;
        }
        flush_text(&mut items, &mut text_runs);
        let Some(generated) = crate::layout::layoutbox::generation::generate_node(arena, child_id)
        else {
            continue;
        };
        if !matches!(
            generated.kind,
            GeneratedBoxKind::Normal | GeneratedBoxKind::Replaced
        ) {
            continue;
        }
        let placement = child.css_attr.get_grid_placements();
        let GridPlacements {
            row_start,
            column_start,
            row_end,
            column_end,
        } = placement;
        let (row_start, row_end) = resolve_axis(row_start, row_end, true, areas.as_deref());
        let (column_start, column_end) =
            resolve_axis(column_start, column_end, false, areas.as_deref());
        let natural = crate::layout::inner_formatting_context::flex::natural_grid_item_size(
            arena,
            child_id,
            context.child(font_size_cells),
            None,
        );
        items.push(GridItemPrep {
            source: GridItemSource::Node(child_id),
            natural_width: natural.0.max(1),
            natural_height: natural.1.max(1),
            row: RawPlacement {
                start: row_start,
                end: row_end,
            },
            column: RawPlacement {
                start: column_start,
                end: column_end,
            },
            align: child.css_attr.get_grid_align_self(),
            justify: child.css_attr.get_grid_justify_self(),
            order: child.css_attr.get_order(),
        });
    }
    flush_text(&mut items, &mut text_runs);
    items
}

/// Bounding rectangle `(row, row_span, column, column_span)` of one named
/// area, hoisted out of the `grid-template-areas` matrix in a single pass.
type AreaBounds = (usize, usize, usize, usize);

/// Resolve one axis's placement pair against the container's area bounds.
///
/// A bare name resolves to the area's edge line (unknown names fall back to
/// `auto`, like a missing implicit line). `span N name` anchors an N-track
/// span at the area's edge — a start edge ends at the area's first line, an
/// end edge starts at the line after the area — and an unknown name degrades
/// to a plain span. An already-definite end line wins over a start-span
/// anchor (the pair then reads as `span N` ending there).
fn resolve_axis(
    start: Option<GridLine>,
    end: Option<GridLine>,
    row_axis: bool,
    areas: Option<&[(Arc<str>, AreaBounds)]>,
) -> (Option<GridLine>, Option<GridLine>) {
    // The 1-based line the area edge names on this axis: start edges name
    // the area's first line, end edges the line after the area.
    let anchor = |name: &Arc<str>, end_edge: bool| -> Option<usize> {
        let (row, row_span, column, column_span) = areas?
            .iter()
            .find(|(area, _)| area == name)
            .map(|(_, bounds)| *bounds)?;
        let (origin, span) = if row_axis {
            (row, row_span)
        } else {
            (column, column_span)
        };
        Some(if end_edge {
            origin + span + 1
        } else {
            origin + 1
        })
    };
    let as_line = |line: usize| Some(GridLine::Line(line as isize));
    let mut start = start;
    let mut end = end;
    match start.take() {
        Some(GridLine::Name(name)) => start = anchor(&name, false).and_then(as_line),
        Some(GridLine::SpanName { span, name }) => {
            if matches!(end, Some(GridLine::Line(_))) {
                // A definite end reads the component as a plain span.
                start = Some(GridLine::Span(span));
            } else {
                match anchor(&name, false) {
                    Some(line) => {
                        start = as_line(line.saturating_sub(span).max(1));
                        end = as_line(line);
                    }
                    None => start = Some(GridLine::Span(span)),
                }
            }
        }
        other => start = other,
    }
    match end.take() {
        Some(GridLine::Name(name)) => end = anchor(&name, true).and_then(as_line),
        // The end-edge span anchor only pins the pair when the start edge
        // did not already resolve to a line.
        Some(GridLine::SpanName { span, name })
            if matches!(start, None | Some(GridLine::Auto) | Some(GridLine::Span(_))) =>
        {
            match anchor(&name, true) {
                Some(line) => {
                    start = as_line(line);
                    end = as_line(line + span);
                }
                None => end = Some(GridLine::Span(span)),
            }
        }
        other => end = other,
    }
    (start, end)
}

/// Collect the bounding rectangle of every named area. The parser
/// guarantees rectangularity, so the first sighting of a name (row-major
/// scan) fixes the start corner and later sightings only extend the ends.
fn collect_area_bounds(areas: &GridAreas) -> Vec<(Arc<str>, AreaBounds)> {
    let mut bounds: Vec<(Arc<str>, AreaBounds)> = Vec::new();
    for (r, row) in areas.cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let Some(name) = cell else {
                continue;
            };
            if let Some((_, area)) = bounds.iter_mut().find(|(area, _)| area == name) {
                area.1 = area.1.max(r);
                area.3 = area.3.max(c);
            } else {
                bounds.push((name.clone(), (r, r, c, c)));
            }
        }
    }
    bounds
        .into_iter()
        .map(|(name, (r0, r1, c0, c1))| (name, (r0, r1 - r0 + 1, c0, c1 - c0 + 1)))
        .collect()
}

#[derive(Clone, Copy)]
enum Axis {
    Row,
    Column,
}

/// Smallest number of tracks needed by positive explicit lines.
fn min_tracks(items: &[GridItemPrep], axis: Axis) -> usize {
    let mut count = 1;
    for item in items {
        let placement = match axis {
            Axis::Row => &item.row,
            Axis::Column => &item.column,
        };
        for line in [&placement.start, &placement.end] {
            if let Some(GridLine::Line(n)) = line
                && *n > 0
            {
                count = count.max(*n as usize);
            }
        }
        if let Some(GridLine::Line(n)) = &placement.start
            && *n > 0
        {
            count = count.max(*n as usize - 1 + placement.span());
        }
    }
    count.max(1)
}

fn explicit_count(tracks: &[GridTrack]) -> usize {
    tracks.len()
}

fn place_item(
    item: &GridItemPrep,
    flow: GridAutoFlow,
    column_count: usize,
    row_count: usize,
    cursor: &mut (usize, usize),
    occupied: &HashSet<(usize, usize)>,
) -> Placed {
    let raw_row = &item.row;
    let raw_column = &item.column;
    let column_span = raw_column.span();
    let row_span = raw_row.span();

    let definite_line = |axis: Axis, line: Option<&GridLine>| -> Option<usize> {
        let Some(GridLine::Line(n)) = line else {
            return None;
        };
        let count = match axis {
            Axis::Row => row_count,
            Axis::Column => column_count,
        };
        Some(resolve_line(*n, count))
    };

    let row_start = definite_line(Axis::Row, raw_row.start.as_ref());
    let column_start = definite_line(Axis::Column, raw_column.start.as_ref());
    let row_end = definite_line(Axis::Row, raw_row.end.as_ref());
    let column_end = definite_line(Axis::Column, raw_column.end.as_ref());

    let row_span = match (&raw_row.end, row_start, row_end) {
        (Some(GridLine::Span(n)), _, _) => (*n).max(1),
        (_, Some(start), Some(end)) => end.saturating_sub(start).max(1),
        _ => row_span,
    };
    let column_span = match (&raw_column.end, column_start, column_end) {
        (Some(GridLine::Span(n)), _, _) => (*n).max(1),
        (_, Some(start), Some(end)) => end.saturating_sub(start).max(1),
        _ => column_span,
    };

    // A definite end with an auto start anchors the span at `end - span`
    // (CSS §8.5); the derived start then behaves like an explicit one.
    let row_start = row_start.or(row_end.map(|end| end.saturating_sub(row_span.max(1))));
    let column_start =
        column_start.or(column_end.map(|end| end.saturating_sub(column_span.max(1))));

    // Both axes definite: explicit items may intentionally overlap.
    if let (Some(row), Some(column)) = (row_start, column_start) {
        return Placed {
            row,
            column,
            row_span,
            column_span,
        };
    }

    // A definite row/column scans the opposite axis from its start.
    if let Some(row) = row_start {
        for column in 0..column_count.saturating_sub(column_span.saturating_sub(1)) {
            if area_free(occupied, row, column, row_span, column_span) {
                return Placed {
                    row,
                    column,
                    row_span,
                    column_span,
                };
            }
        }
        return Placed {
            row,
            column: column_count,
            row_span,
            column_span,
        };
    }
    if let Some(column) = column_start {
        for row in 0..row_count.saturating_sub(row_span.saturating_sub(1)) {
            if area_free(occupied, row, column, row_span, column_span) {
                return Placed {
                    row,
                    column,
                    row_span,
                    column_span,
                };
            }
        }
        return Placed {
            row: row_count,
            column,
            row_span,
            column_span,
        };
    }

    // Fully automatic placement. A sparse cursor never moves backwards; a
    // dense item restarts at the grid origin and may fill an earlier hole.
    let append_main = if flow.row { row_count } else { column_count };
    let start_main = if flow.dense {
        0
    } else if flow.row {
        cursor.0
    } else {
        cursor.1
    };
    for main in start_main..=append_main.saturating_add(occupied.len()) {
        let cross_start = if flow.dense || main != start_main {
            0
        } else if flow.row {
            cursor.1
        } else {
            cursor.0
        };
        let cross_limit = if flow.row {
            column_count.saturating_sub(column_span.saturating_sub(1))
        } else {
            row_count.saturating_sub(row_span.saturating_sub(1))
        };
        for cross in cross_start..cross_limit {
            let (row, column) = if flow.row {
                (main, cross)
            } else {
                (cross, main)
            };
            if area_free(occupied, row, column, row_span, column_span) {
                *cursor = if flow.row {
                    (row, column)
                } else {
                    (column, row)
                };
                return Placed {
                    row,
                    column,
                    row_span,
                    column_span,
                };
            }
        }
    }

    // No free slot inside the current implicit bound: append a track. The
    // final count pass grows the grid from this position.
    let (row, column) = if flow.row {
        (append_main, 0)
    } else {
        (0, append_main)
    };
    *cursor = if flow.row {
        (row, column)
    } else {
        (column, row)
    };
    Placed {
        row,
        column,
        row_span,
        column_span,
    }
}

fn resolve_line(line: isize, track_count: usize) -> usize {
    if line > 0 {
        (line as usize - 1).min(track_count.max(1))
    } else {
        let from_end = track_count
            .checked_add(1)
            .and_then(|lines| lines.checked_sub(line.unsigned_abs()))
            .unwrap_or(0);
        from_end.min(track_count.max(1))
    }
}

fn area_free(
    occupied: &HashSet<(usize, usize)>,
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
) -> bool {
    for r in row..row.saturating_add(row_span) {
        for c in column..column.saturating_add(column_span) {
            if occupied.contains(&(r, c)) {
                return false;
            }
        }
    }
    true
}

fn mark_occupied(occupied: &mut HashSet<(usize, usize)>, placed: Placed) {
    for r in placed.row..placed.row.saturating_add(placed.row_span) {
        for c in placed.column..placed.column.saturating_add(placed.column_span) {
            occupied.insert((r, c));
        }
    }
}

/// Accumulate the intrinsic size contributed to every auto track.
fn track_hints<'a, I, N, P>(items: I, natural: N, position: P, track_count: usize) -> Vec<usize>
where
    I: Iterator<Item = (&'a GridItemPrep, &'a Placed)>,
    N: Fn(&GridItemPrep) -> usize,
    P: Fn(&Placed) -> (usize, usize),
{
    let mut hints = vec![0usize; track_count];
    for (item, placed) in items {
        let (start, span) = position(placed);
        let contribution = natural(item).max(1);
        let each = if span <= 1 {
            contribution
        } else {
            // Multi-span content sizing is intentionally conservative: split
            // the contribution equally instead of charging every track fully.
            (contribution / span).max(1)
        };
        for track in start..start.saturating_add(span) {
            if let Some(hint) = hints.get_mut(track) {
                *hint = (*hint).max(each);
            }
        }
    }
    hints
}

/// Fallback spec for implicit tracks when no `grid-auto-*` list exists.
const AUTO_TRACK: GridTrack = GridTrack::Auto;

/// Explicit track at `index`, or the cycled `grid-auto-*` entry beyond the
/// explicit list.
fn track_spec<'a>(
    explicit: &'a [GridTrack],
    implicit: &'a [GridTrack],
    index: usize,
) -> &'a GridTrack {
    if let Some(spec) = explicit.get(index) {
        return spec;
    }
    let list: &[GridTrack] = if implicit.is_empty() {
        std::slice::from_ref(&AUTO_TRACK)
    } else {
        implicit
    };
    &list[(index - explicit.len()) % list.len()]
}

fn track_sizes(
    explicit: &[GridTrack],
    implicit: &[GridTrack],
    hints: &[usize],
    definite_size: Option<usize>,
    gap: usize,
    font: crate::html::css_parse::FontContext,
    viewport: crate::html::css_parse::Viewport,
) -> Vec<usize> {
    // Implicit tracks cycle through the `grid-auto-*` list; an empty list
    // behaves as a single `auto`.
    let count = hints.len().max(explicit.len()).max(1);
    let gap_total = if count > 1 {
        gap.saturating_mul(count - 1)
    } else {
        0
    };
    let resolve = |length: crate::html::css_parse::Length, hint: usize| -> usize {
        length
            .resolve(crate::html::css_parse::LengthContext {
                containing: definite_size.unwrap_or(hint),
                font,
                viewport,
            })
            .max(1)
    };
    // Pass 1: base sizes (the min side / content hint) and growth limits
    // (the max side). Flexible tracks wait for the distribution pass.
    // Specs are borrowed, not cloned, per track.
    let tracks: Vec<&GridTrack> = (0..count)
        .map(|index| track_spec(explicit, implicit, index))
        .collect();
    let mut sizes = Vec::with_capacity(count);
    let mut limits = vec![0usize; count];
    let mut total_fr = 0.0f32;
    for (index, spec) in tracks.iter().enumerate() {
        let hint = hints.get(index).copied().unwrap_or(1).max(1);
        let (base, limit, fr) = match *spec {
            GridTrack::Auto => (hint, hint, 0.0),
            GridTrack::Length(length) => {
                let size = resolve(*length, hint);
                (size, size, 0.0)
            }
            GridTrack::Fr(n) => (0, 0, (*n).max(0.0)),
            GridTrack::MinMax { min, max } => {
                let base = match &**min {
                    // Parse rejects flexible/nested mins; the content hint is
                    // the safe fallback if one still reaches layout.
                    GridTrack::Auto
                    | GridTrack::Fr(_)
                    | GridTrack::MinMax { .. }
                    | GridTrack::AutoRepeat { .. } => hint,
                    GridTrack::Length(length) => resolve(*length, hint),
                };
                let limit = match &**max {
                    GridTrack::Length(length) => resolve(*length, hint).max(base),
                    GridTrack::Auto
                    | GridTrack::Fr(_)
                    | GridTrack::MinMax { .. }
                    | GridTrack::AutoRepeat { .. } => base,
                };
                let fr = match &**max {
                    GridTrack::Fr(n) => (*n).max(0.0),
                    _ => 0.0,
                };
                (base, limit, fr)
            }
            // Auto-repeats are expanded before sizing; treat any survivor as
            // a content-sized track.
            GridTrack::AutoRepeat { .. } => (hint, hint, 0.0),
        };
        total_fr += fr;
        sizes.push(base);
        limits[index] = limit;
    }

    if let Some(available) = definite_size {
        // Maximize tracks: grow tracks toward their limits before the `fr`
        // pass claims the leftover space.
        let mut free = available.saturating_sub(
            sizes
                .iter()
                .copied()
                .fold(0usize, usize::saturating_add)
                .saturating_add(gap_total),
        );
        while free > 0 {
            let growable: Vec<usize> = (0..count).filter(|&i| limits[i] > sizes[i]).collect();
            if growable.is_empty() {
                break;
            }
            let share = free / growable.len();
            if share == 0 {
                break;
            }
            let mut distributed = 0usize;
            for index in growable {
                let add = share.min(limits[index] - sizes[index]);
                sizes[index] += add;
                distributed += add;
            }
            if distributed == 0 {
                break;
            }
            free -= distributed;
        }
        if total_fr > f32::EPSILON {
            // CSS §12.7 "find the size of an fr": the space to fill is what
            // the non-flexible tracks leave; flexible tracks share it by
            // weight, but a track whose base exceeds its share freezes at
            // the base and its space is redistributed among the rest
            // (restart until no violation). Cumulative rounding keeps the
            // integer shares summing to the leftover space.
            let non_flexible: usize = sizes
                .iter()
                .enumerate()
                .filter(|(index, _)| tracks[*index].fr_weight() == 0.0)
                .map(|(_, size)| *size)
                .fold(0usize, usize::saturating_add);
            let mut leftover = available.saturating_sub(non_flexible.saturating_add(gap_total));
            let mut flexible: Vec<(usize, f32)> = (0..count)
                .filter(|index| tracks[*index].fr_weight() > 0.0)
                .map(|index| (index, tracks[index].fr_weight()))
                .collect();
            loop {
                let weight_total: f32 = flexible.iter().map(|(_, weight)| weight).sum();
                if weight_total <= f32::EPSILON || leftover == 0 {
                    break;
                }
                let fr = leftover as f32 / weight_total;
                let violators: Vec<usize> = flexible
                    .iter()
                    .enumerate()
                    .filter_map(|(position, (index, weight))| {
                        (fr * weight < sizes[*index] as f32).then_some(position)
                    })
                    .collect();
                if violators.is_empty() {
                    break;
                }
                for position in violators.into_iter().rev() {
                    let (index, _) = flexible.remove(position);
                    leftover = leftover.saturating_sub(sizes[index]);
                }
            }
            let weight_total: f32 = flexible.iter().map(|(_, weight)| weight).sum();
            let mut allocated = 0.0f32;
            for (index, weight) in &flexible {
                let previous = allocated;
                allocated += leftover as f32 * (weight / weight_total);
                let share = (allocated.round() as usize).saturating_sub(previous.round() as usize);
                sizes[*index] = (sizes[*index]).max(share);
            }
        }
    }
    for size in &mut sizes {
        *size = (*size).max(1);
    }
    sizes
}

/// Expand `repeat(auto-fill | auto-fit, …)` in an explicit template into
/// concrete tracks. The repetition count is the largest number whose tracks
/// (counted at their definite width: the max side if definite, else the
/// min side) and inner gaps fit the available size; a list without definite
/// widths, or an indefinite container, repeats exactly once. Returns the
/// expanded list plus `(start, end, is_auto_fit)` segment ranges.
fn expand_auto_repeats(
    tracks: &[GridTrack],
    definite: Option<usize>,
    gap: usize,
    font: crate::html::css_parse::FontContext,
    viewport: crate::html::css_parse::Viewport,
) -> (Vec<GridTrack>, Vec<(usize, usize, bool)>) {
    let mut out = Vec::new();
    let mut segments = Vec::new();
    for spec in tracks {
        let GridTrack::AutoRepeat { fit, tracks: inner } = spec else {
            out.push(spec.clone());
            continue;
        };
        let start = out.len();
        let count = auto_repeat_count(inner, definite, gap, font, viewport);
        for _ in 0..count {
            out.extend(inner.iter().cloned());
        }
        segments.push((start, out.len(), *fit));
    }
    (out, segments)
}

/// Definite counting width of one track inside an auto-repeat, if any: its
/// max side when that is a definite length, else its min side.
fn repeat_counting_width(
    spec: &GridTrack,
    available: usize,
    font: crate::html::css_parse::FontContext,
    viewport: crate::html::css_parse::Viewport,
) -> Option<usize> {
    let resolve = |length: &crate::html::css_parse::Length| {
        length
            .resolve(crate::html::css_parse::LengthContext {
                containing: available,
                font,
                viewport,
            })
            .max(1)
    };
    match spec {
        GridTrack::Length(length) => Some(resolve(length)),
        GridTrack::MinMax { min, max } => match &**max {
            GridTrack::Length(length) => Some(resolve(length)),
            _ => match &**min {
                GridTrack::Length(length) => Some(resolve(length)),
                _ => None,
            },
        },
        _ => None,
    }
}

/// Repetition count for one auto-repeat segment (minimum one).
fn auto_repeat_count(
    inner: &[GridTrack],
    definite: Option<usize>,
    gap: usize,
    font: crate::html::css_parse::FontContext,
    viewport: crate::html::css_parse::Viewport,
) -> usize {
    let Some(available) = definite else {
        return 1;
    };
    let Some(widths) = inner
        .iter()
        .map(|spec| repeat_counting_width(spec, available, font, viewport))
        .collect::<Option<Vec<_>>>()
    else {
        return 1;
    };
    let width_total: usize = widths.iter().sum();
    if width_total == 0 {
        return 1;
    }
    // Largest repetition count whose N * width + (N - 1) * gap fits.
    (available.saturating_add(gap) / width_total.saturating_add(gap)).max(1)
}

/// Drop the trailing tracks of a tail `auto-fit` segment that no item
/// spans, so the remaining tracks can grow into the freed space. Segments
/// followed by other tracks keep every repetition (auto-fill behavior).
fn collapse_fit_tail(
    tracks: &mut Vec<GridTrack>,
    segments: &[(usize, usize, bool)],
    placements: &[Placed],
    axis: Axis,
) {
    for &(start, end, fit) in segments.iter().rev() {
        if !fit || end != tracks.len() {
            continue;
        }
        let spans = |index: usize| {
            placements.iter().any(|placed| {
                let (origin, span) = match axis {
                    Axis::Column => (placed.column, placed.column_span),
                    Axis::Row => (placed.row, placed.row_span),
                };
                origin <= index && index < origin.saturating_add(span)
            })
        };
        let mut end = end;
        while end > start && !spans(end - 1) {
            end -= 1;
        }
        tracks.truncate(end);
    }
}

/// Union the areas covered by one item across consecutive tracks. Gaps
/// belong to the item area, so a two-row item stretches through the row gap.
fn spanned_area(areas: &[TrackArea], start: usize, span: usize) -> Option<TrackArea> {
    let first = areas.get(start)?;
    let last_index = start.saturating_add(span.max(1)).saturating_sub(1);
    let last = areas.get(last_index).unwrap_or(first);
    Some(TrackArea {
        origin: first.origin,
        size: last
            .origin
            .saturating_sub(first.origin)
            .saturating_add(last.size.max(1)),
    })
}

fn track_areas(sizes: &[usize], origin: usize, gap: usize) -> Vec<TrackArea> {
    let mut areas = Vec::with_capacity(sizes.len());
    let mut cursor = origin;
    for (index, size) in sizes.iter().enumerate() {
        areas.push(TrackArea {
            origin: cursor,
            size: (*size).max(1),
        });
        cursor = cursor.saturating_add((*size).max(1));
        if index + 1 < sizes.len() {
            cursor = cursor.saturating_add(gap);
        }
    }
    areas
}

fn align_content_to_justify(value: AlignContent) -> JustifyContent {
    match value {
        AlignContent::FlexStart | AlignContent::Stretch => JustifyContent::FlexStart,
        AlignContent::FlexEnd => JustifyContent::FlexEnd,
        AlignContent::Center => JustifyContent::Center,
        AlignContent::SpaceBetween => JustifyContent::SpaceBetween,
        AlignContent::SpaceAround => JustifyContent::SpaceAround,
        AlignContent::SpaceEvenly => JustifyContent::SpaceEvenly,
    }
}

fn track_alignment_offset(alignment: JustifyContent, available: usize, content: usize) -> usize {
    let free = available.saturating_sub(content);
    match alignment {
        JustifyContent::FlexEnd => free,
        JustifyContent::Center => free / 2,
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_grid_item(
    arena: &NodeArena,
    item: &GridItemPrep,
    column_area: TrackArea,
    row_area: TrackArea,
    container_align: AlignItems,
    container_justify_items: AlignItems,
    container_height: Option<usize>,
    content_width: usize,
    context: LayoutContext,
    container_id: NodeId,
) -> Option<LayoutBox> {
    let align = item
        .align
        .or(Some(container_align))
        .map(grid_align)
        .unwrap_or(AlignItems::Stretch);
    let justify = grid_align(item.justify.unwrap_or(container_justify_items));
    let align_stretch = align == AlignItems::Stretch;
    let justify_stretch = justify == AlignItems::Stretch;

    let css = match &item.source {
        GridItemSource::Node(id) => Some(&arena[*id].css_attr),
        GridItemSource::Text(_) => None,
    };
    let mut item_width = if justify_stretch {
        column_area.size.max(1)
    } else {
        item.natural_width.max(1).min(column_area.size.max(1))
    };
    let mut item_height = if align_stretch {
        row_area.size.max(1)
    } else {
        item.natural_height.max(1).min(row_area.size.max(1))
    };
    if let Some(css) = css {
        if css
            .get_width(content_width, context.font, context.viewport)
            .is_some()
        {
            item_width = item.natural_width.max(1).min(column_area.size.max(1));
        }
        if css
            .get_height(container_height, context.font, context.viewport)
            .is_some()
        {
            item_height = item.natural_height.max(1).min(row_area.size.max(1));
        }
    }

    let item_x = alignment_origin(justify, column_area, item_width);
    let item_y = alignment_origin(align, row_area, item_height);
    let rect = Rect::new(item_x, item_y, item_width.max(1), item_height.max(1));
    match &item.source {
        GridItemSource::Node(id) => flow_dispatch(
            arena,
            *id,
            BoxConstraint {
                x: item_x,
                width: content_width,
            },
            &mut Cursor {
                y: item_y,
                prev_bottom_margin: 0,
            },
            context,
            Some(rect),
        ),
        GridItemSource::Text(text) => Some(layout_anonymous_grid_text(
            text,
            rect,
            container_id,
            align_stretch,
            justify_stretch,
        )),
    }
}

fn grid_align(value: AlignItems) -> AlignItems {
    match value {
        AlignItems::Baseline => AlignItems::FlexStart,
        other => other,
    }
}

fn alignment_origin(alignment: AlignItems, area: TrackArea, item_size: usize) -> usize {
    let free = area.size.saturating_sub(item_size);
    match alignment {
        AlignItems::FlexEnd => area.origin.saturating_add(free),
        AlignItems::Center => area.origin.saturating_add(free / 2),
        _ => area.origin,
    }
}

fn layout_anonymous_grid_text(
    text: &AnonymousGridText,
    rect: Rect,
    container_node_id: NodeId,
    stretch_height: bool,
    stretch_width: bool,
) -> LayoutBox {
    let width = if stretch_width {
        rect.width.max(1)
    } else {
        item_natural_text_width(&text.runs)
            .min(rect.width.max(1))
            .max(1)
    };
    let wrapped = wrap_runs(
        &text.runs,
        width,
        text.font_size_cells,
        text.white_space,
        text.word_break,
        text.overflow_wrap,
        0,
    );
    let has_lines = !wrapped.is_empty() && wrapped.iter().any(|line| !line.is_empty());
    let inline_height = if has_lines {
        wrapped.len().saturating_mul(text.line_height_cells.max(1))
    } else {
        0
    };
    let height = if stretch_height {
        rect.height.max(inline_height.max(1))
    } else {
        inline_height
            .max(1)
            .min(rect.height.max(inline_height.max(1)))
    };
    let final_rect = Rect::new(rect.x, rect.y, width, height);
    let paint_style = PaintStyle {
        box_edges: crate::html::box_edges::BoxEdges::default(),
        runs: has_lines.then_some(wrapped),
        column_layout: None,
        font_size_cells: text.font_size_cells,
        line_height_cells: text.line_height_cells,
        border_radius: crate::html::css_parse::CornerRadii::default(),
        fill_bg: false,
        spacing_after: 0,
        text_align: crate::html::css_parse::TextAlign::Left,
        text_indent: 0,
        scrollbar: None,
    };
    LayoutBox::new_normal(
        container_node_id,
        final_rect,
        paint_style,
        OuterDisplay::BlockLevel { container: true },
        Vec::new(),
    )
}

fn item_natural_text_width(runs: &[InlineRun]) -> usize {
    runs.iter()
        .filter(|run| !run.force_break)
        .map(|run| {
            run.text
                .graphemes(true)
                .map(crate::graphics::grapheme_width)
                .sum::<usize>()
        })
        .sum::<usize>()
        .max(1)
}

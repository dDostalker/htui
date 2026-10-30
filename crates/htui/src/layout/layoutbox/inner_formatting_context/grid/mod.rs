//! CSS Grid formatting context.
//!
//! Implements the engine's terminal-grid subset:
//!
//! - display: grid / inline-grid
//! - track lists with auto, lengths/percentages, fr, and repeat()
//! - explicit line-number/span placement and automatic row/column placement
//!   (including dense)
//! - row/column gaps and per-item start/end/center/stretch alignment
//!
//! Named lines/areas, minmax(), fit-content functions and subgrid are not
//! part of this first implementation; unsupported declarations are rejected
//! at the CSS value layer rather than guessed.

use std::collections::HashSet;

use crate::html::css_parse::{
    AlignContent, AlignItems, GridAutoFlow, GridLine, GridPlacements, GridTrack, JustifyContent,
    OverflowWrap, WhiteSpace, WordBreak,
};
use crate::layout::{
    BoxConstraint, Cursor, InlineRun, LayoutBox, LayoutContext, OuterDisplay, PaintStyle, Rect,
    inner_formatting_context::flow::{apply_horizontal_auto_margins, constrained_block_width},
    layoutbox::{
        flatten_contents_children, generation::GeneratedBoxKind, text_node_run, translate_subtree,
        visible_children, wrap_runs,
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

#[derive(Clone, Copy)]
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
        match (self.start, self.end) {
            (Some(GridLine::Span(n)), _) | (_, Some(GridLine::Span(n))) => n.max(1),
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

    let template = css.get_grid_template(child_context.font, child_context.viewport);
    let auto_columns = css.get_grid_auto_columns();
    let auto_rows = css.get_grid_auto_rows();
    let auto_flow = css.get_grid_auto_flow();
    let gap = css.get_gap(child_context.font, child_context.viewport);
    let align_items = css.get_align_items();
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
    let mut column_count = explicit_count(&template.columns)
        .max(min_tracks(&items, Axis::Column))
        .max(1);
    let mut row_count = explicit_count(&template.rows)
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

    for placed in &placements {
        column_count = column_count
            .max(placed.column.saturating_add(placed.column_span))
            .max(explicit_count(&template.columns));
        row_count = row_count
            .max(placed.row.saturating_add(placed.row_span))
            .max(explicit_count(&template.rows));
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
        &template.columns,
        auto_columns,
        &column_hints,
        Some(content_width),
        gap.column,
        child_context.font,
        child_context.viewport,
    );
    let rows = track_sizes(
        &template.rows,
        auto_rows,
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
        .sum::<usize>()
        .saturating_add(gap.column.saturating_mul(columns.len().saturating_sub(1)));
    let grid_height: usize = rows
        .iter()
        .sum::<usize>()
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
            placed,
            column_area,
            row_area,
            align_items,
            align_content,
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
        let (row_start, row_end) = placement_rows(placement);
        let (column_start, column_end) = placement_columns(placement);
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

fn placement_rows(placement: GridPlacements) -> (Option<GridLine>, Option<GridLine>) {
    (placement.row_start, placement.row_end)
}

fn placement_columns(placement: GridPlacements) -> (Option<GridLine>, Option<GridLine>) {
    (placement.column_start, placement.column_end)
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
            Axis::Row => item.row,
            Axis::Column => item.column,
        };
        for line in [placement.start, placement.end] {
            if let Some(GridLine::Line(n)) = line
                && n > 0
            {
                count = count.max(n as usize);
            }
        }
        if let Some(GridLine::Line(n)) = placement.start
            && n > 0
        {
            count = count.max(n as usize - 1 + placement.span());
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
    let raw_row = item.row;
    let raw_column = item.column;
    let column_span = raw_column.span();
    let row_span = raw_row.span();

    let definite_start = |axis: Axis, raw: RawPlacement| -> Option<usize> {
        match raw.start {
            Some(GridLine::Line(n)) => {
                let count = match axis {
                    Axis::Row => row_count,
                    Axis::Column => column_count,
                };
                Some(resolve_line(n, count))
            }
            _ => None,
        }
    };
    let definite_end = |axis: Axis, raw: RawPlacement| -> Option<usize> {
        match raw.end {
            Some(GridLine::Line(n)) => {
                let count = match axis {
                    Axis::Row => row_count,
                    Axis::Column => column_count,
                };
                Some(resolve_line(n, count))
            }
            _ => None,
        }
    };

    let row_start = definite_start(Axis::Row, raw_row);
    let column_start = definite_start(Axis::Column, raw_column);
    let row_end = definite_end(Axis::Row, raw_row);
    let column_end = definite_end(Axis::Column, raw_column);

    let row_span = match (raw_row.end, row_start, row_end) {
        (Some(GridLine::Span(n)), _, _) => n.max(1),
        (_, Some(start), Some(end)) => end.saturating_sub(start).max(1),
        _ => row_span,
    };
    let column_span = match (raw_column.end, column_start, column_end) {
        (Some(GridLine::Span(n)), _, _) => n.max(1),
        (_, Some(start), Some(end)) => end.saturating_sub(start).max(1),
        _ => column_span,
    };

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

fn track_sizes(
    explicit: &[GridTrack],
    implicit: GridTrack,
    hints: &[usize],
    definite_size: Option<usize>,
    gap: usize,
    font: crate::html::css_parse::FontContext,
    viewport: crate::html::css_parse::Viewport,
) -> Vec<usize> {
    let track = |index: usize| explicit.get(index).copied().unwrap_or(implicit);
    let count = hints.len().max(explicit.len()).max(1);
    let gap_total = if count > 1 {
        gap.saturating_mul(count - 1)
    } else {
        0
    };
    let mut sizes = Vec::with_capacity(count);
    let mut total_fr = 0.0f32;
    for index in 0..count {
        let hint = hints.get(index).copied().unwrap_or(1).max(1);
        let size = match track(index) {
            GridTrack::Auto => hint,
            GridTrack::Length(length) => length
                .resolve(crate::html::css_parse::LengthContext {
                    containing: definite_size.unwrap_or(hint),
                    font,
                    viewport,
                })
                .max(1),
            GridTrack::Fr(n) => {
                total_fr += n.max(0.0);
                0
            }
        };
        sizes.push(size);
    }

    let definite_size = definite_size.filter(|_| total_fr > 0.0);
    if let Some(available) = definite_size {
        let fixed: usize = sizes.iter().sum::<usize>().saturating_add(gap_total);
        let free = available.saturating_sub(fixed);
        if total_fr > f32::EPSILON {
            for (index, size) in sizes.iter_mut().enumerate() {
                if let GridTrack::Fr(n) = track(index)
                    && n > 0.0
                {
                    let share = (free as f32 * (n / total_fr)).round() as usize;
                    *size = (*size).max(share);
                }
            }
        }
    }
    for size in &mut sizes {
        *size = (*size).max(1);
    }
    sizes
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
    placed: &Placed,
    column_area: TrackArea,
    row_area: TrackArea,
    container_align: AlignItems,
    _container_align_content: AlignContent,
    container_height: Option<usize>,
    content_width: usize,
    context: LayoutContext,
    container_id: NodeId,
) -> Option<LayoutBox> {
    let _ = placed;
    let align = item
        .align
        .or(Some(container_align))
        .map(grid_align)
        .unwrap_or(AlignItems::Stretch);
    let justify = grid_align(item.justify.unwrap_or(AlignItems::Stretch));
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

// Track-level refinements can reuse the existing subtree translation helper.
#[allow(dead_code)]
fn _translate_child(child: &mut LayoutBox, dx: i64, dy: i64) {
    translate_subtree(child, dx, dy);
}

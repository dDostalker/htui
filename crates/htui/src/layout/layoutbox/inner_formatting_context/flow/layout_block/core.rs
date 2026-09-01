//! Shared body of every block-level layout function.
//!
//! [`layout_block`] is the single seam where every per-element handler
//! converges. It walks the same pipeline (resolve width → compute edges →
//! allocate rect → render children → reserve next row) and the element
//! only contributes its options. Keeping the pipeline in one place means
//! adding a new block element only requires a few lines that pick the
//! right [`LayoutBlockOptions`].

use crate::{
    html::box_edges::{Border, BorderSides, BorderStyle, BoxEdges, EdgeSizes},
    html::css_parse::{
        BoxSizing, ColumnFill, ColumnsResolved, OverflowWrap, Position, WhiteSpace, WordBreak,
    },
    layout::{
        BoxConstraint, ColumnLayout, Cursor, InlineRun, LayoutBox, LayoutContext, PaintStyle, Rect,
        inner_formatting_context::flow::layout_children,
        layoutbox::{collect_runs, visible_children, wrap_runs},
    },
    tdom::node::{Node, NodeArena, NodeId, effective_font_size_cells},
    graphics::{FontStyle, block_style},
};

use crossterm::style::Color;

use crate::html::css_parse::resolve_line_height_cells;

use super::geometry::{
    OuterGeometry, block_left, build_border_rect, compute_outer_geometry, constrained_block_width,
    reserve_border_box,
};
use super::options::LayoutBlockOptions;

/// Shared body of every block-level layout function.
///
/// Resolves the CSS `width`, padding / border / margin, and the inline
/// `runs`, then either reserves a border-box in normal block flow or honors
/// the border-box supplied by a parent formatting context. The element-specific
/// differences are passed in via `opts` so the boilerplate (collect_runs,
/// child layout, and box construction) lives in one place.
///
/// `context` carries the parent's effective font-size, the root font-size,
/// and the terminal viewport so `em` / `rem` / `vw` / `vh` lengths inside the
/// element resolve against the active layout context. The element's own
/// font-size is computed via [`effective_font_size_cells`] and forwarded to
/// nested children via `context.child(...)` so the chain remains consistent.
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
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> LayoutBox {
    let own_font_size_cells =
        effective_font_size_cells(arena, node_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);

    let edges: BoxEdges =
        node.css_attr
            .get_box_edges(containing.width, child_context.font, child_context.viewport);
    // Layer the browser UA default margins (`opts.ua_margins`) on top of
    // the author CSS edges. Each side keeps its UA value only while neither
    // the `margin` shorthand nor that side's longhand is declared by the
    // author — the same precedence an author stylesheet has over the UA
    // stylesheet in the browser cascade.
    let edges = merge_ua_margins(node, edges, opts.ua_margins);
    let edges = merge_ua_border(node, edges, opts.ua_border);
    let sizing = node.css_attr.get_box_sizing();
    let line_height_cells = resolve_line_height_cells(
        node.css_attr.get_line_height(),
        own_font_size_cells,
        child_context.font,
        child_context.viewport,
    );
    // The available width after margins / indent — shared by the width
    // resolution and the min/max-height clamp below.
    let available = containing
        .width
        .saturating_sub(opts.indent)
        .saturating_sub(edges.margin.horizontal());

    let (geom, preallocated_height) = match pre_allocated_rect {
        Some(rect) => {
            let inner_h = edges.inner_horizontal();
            let content_constraint = BoxConstraint {
                x: rect
                    .x
                    .saturating_add(edges.border.left.thickness())
                    .saturating_add(edges.padding.left),
                width: rect.width.saturating_sub(inner_h).max(1),
            };
            (
                OuterGeometry {
                    outer_x: rect.x,
                    outer_y: rect.y,
                    outer_width: rect.width,
                    content_constraint,
                },
                Some(rect.height),
            )
        }
        None => {
            // Collapse this block's top margin with the previous in-flow
            // sibling's bottom margin (CSS vertical margin collapsing: the
            // larger wins).
            cursor.collapse_and_skip(edges.margin.top);
            let outer_y_top = cursor.y;
            let block_width = constrained_block_width(
                &node.css_attr,
                available,
                edges,
                child_context.font,
                child_context.viewport,
            );
            let mut edges = edges;
            // Horizontal `margin: auto` distributes the containing block's
            // free space: two auto margins center the box, a single one
            // absorbs the whole remainder. `available` already excluded the
            // fixed (non-auto) margins, so the free space is simply the
            // leftover width. Vertical auto margins stay 0 in block flow.
            let margin_auto = node.css_attr.get_margin_auto();
            if margin_auto.left || margin_auto.right {
                let free = available.saturating_sub(block_width);
                match (margin_auto.left, margin_auto.right) {
                    (true, true) => edges.margin.left = free / 2,
                    (true, false) => edges.margin.left = free,
                    _ => {}
                }
            }
            let x = block_left(containing) + opts.indent;
            let geom = compute_outer_geometry(
                BoxConstraint {
                    x,
                    width: block_width,
                },
                block_width,
                edges,
                outer_y_top,
            );
            (geom, None)
        }
    };
    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let mut block_font_style = block_style(&node.css_attr);
    // User-agent default: `<address>` renders italic. Anything the author
    // (or an ancestor rule) declared for `font-style` takes precedence —
    // `address { font-style: normal }` is honoured (same cascade-precedence
    // rule the `<em>` UA default follows).
    if opts.ua_italic && !node.css_attr.is_set("font-style") {
        block_font_style |= FontStyle::ITALIC;
    }
    // `<hr>` has no inline content of its own; skip the (empty) walk.
    let body_runs = if opts.hr_rule {
        Vec::new()
    } else {
        collect_runs(arena, node_id, node, block_fg, None, block_font_style)
    };
    let fill_bg = block_bg.is_some();

    let child_containing = match opts.child_containing_shift {
        Some((x_off, w_delta)) => BoxConstraint {
            x: geom.content_constraint.x.saturating_add(x_off),
            width: if w_delta >= 0 {
                geom.content_constraint
                    .width
                    .saturating_add(w_delta as usize)
                    .max(1)
            } else {
                geom.content_constraint
                    .width
                    .saturating_sub((-w_delta) as usize)
                    .max(1)
            },
        },
        None => geom.content_constraint,
    };
    // The marker / hash prefix belongs to the first visual line. Prepend
    // it to the flat run list so the line-breaker treats it as part of
    // the first "word" group and renders it on the top row only.
    let mut all_runs = if opts.hr_rule {
        // `<hr>` paints a horizontal rule: one `─` glyph per content cell,
        // synthesised at layout time so it always spans the available width.
        let width = geom.content_constraint.width.max(1);
        vec![InlineRun::new(
            "─".repeat(width),
            block_font_style,
            block_fg,
            block_bg,
        )]
    } else if let Some(ratio) = opts.bar_fill {
        // `<progress>` / `<meter>` paint a terminal bar: `█` for the filled
        // part, `░` for the rest, spanning the content-box width. The filled
        // cells carry `bar_color` (or the element fg); the empty cells keep
        // the element fg so the whole run reads as one widget.
        let width = geom.content_constraint.width.max(1);
        let filled = ((width as f32) * ratio.clamp(0.0, 1.0)).round() as usize;
        let mut run_text = "█".repeat(filled.min(width));
        run_text.push_str(&"░".repeat(width.saturating_sub(filled)));
        vec![InlineRun::new(
            run_text,
            block_font_style,
            opts.bar_color.or(block_fg),
            block_bg,
        )]
    } else {
        body_runs
    };
    if let Some(prefix) = opts.prefix.clone() {
        all_runs.insert(0, prefix);
    }
    if let Some(suffix) = opts.suffix.clone() {
        all_runs.push(suffix);
    }

    // Wrap the inline content to the content-box width. The wrapped
    // line count drives the rect height; the per-line run list is
    // what the paint phase draws.
    let content_top = geom.outer_y + edges.border.top.thickness() + edges.padding.top;
    let white_space = node.css_attr.get_white_space(&node.node_type);
    let word_break = node.css_attr.get_word_break();
    let overflow_wrap = node.css_attr.get_overflow_wrap();

    // Multi-column layout (CSS Multi-column Level 1): when `column-count` /
    // `column-width` resolve to more than one column, the inline lines and
    // the block children fragment into side-by-side columns instead of a
    // single column. The multicol branch produces the same outputs the
    // single-column path below does (wrapped lines, children, content
    // height) plus the per-column text table handed to the paint phase.
    let columns = node.css_attr.get_columns_resolved(
        geom.content_constraint.width,
        child_context.font,
        child_context.viewport,
    );
    // `column-fill: auto` fills each column up to the container's *explicit*
    // height before moving on; when no height is declared the spec treats
    // `auto` as `balance`, so this is only consulted for the auto case.
    let fixed_content_height = columns.as_ref().and_then(|_| {
        node.css_attr
            .get_height(None, child_context.font, child_context.viewport)
            .map(|h| {
                let inner_v = edges.padding.vertical() + edges.border.vertical_thickness();
                match sizing {
                    BoxSizing::BorderBox => h.saturating_sub(inner_v),
                    BoxSizing::ContentBox => h,
                }
            })
    });
    let mut column_layout: Option<ColumnLayout> = None;
    let (wrapped_lines, children, content_height) = if let Some(spec) = &columns {
        let (layout, children, height) = layout_multicol(
            arena,
            node_id,
            all_runs,
            geom,
            spec,
            content_top,
            own_font_size_cells,
            line_height_cells,
            white_space,
            word_break,
            overflow_wrap,
            block_fg,
            fixed_content_height,
            child_context,
        );
        column_layout = layout;
        (Vec::new(), children, height)
    } else {
        let wrap_width = geom.content_constraint.width.max(1);
        let wrapped_lines = if all_runs.is_empty() {
            Vec::new()
        } else {
            wrap_runs(
                &all_runs,
                wrap_width,
                own_font_size_cells,
                white_space,
                word_break,
                overflow_wrap,
            )
        };
        let inline_rows = wrapped_lines.len().saturating_mul(line_height_cells.max(1));

        // Block children begin BELOW the box's own inline text: in CSS the
        // inline content forms its own line(s) first and block-level children
        // start on a fresh line underneath. Starting the child cursor at the
        // content top would paint the first child's row over the parent's own
        // text (e.g. `<li>Nested list<ul>…</ul></li>` and
        // `<div>text<p>child</p></div>`).
        let mut child_cursor = Cursor {
            y: content_top.saturating_add(inline_rows),
            prev_bottom_margin: 0,
        };
        let block_children = visible_children(arena, node_id);
        let children = layout_children(
            arena,
            &block_children,
            child_containing,
            &mut child_cursor,
            child_context,
        );

        let children_rows = child_cursor
            .y
            .saturating_sub(content_top.saturating_add(inline_rows));
        let content_height = children_rows.saturating_add(inline_rows);
        (wrapped_lines, children, content_height)
    };

    let outer_height = match preallocated_height {
        Some(height) => {
            // A pre-allocated rect (flex item placement) fixes the item's
            // border-box height, but it must never clip the item's own
            // content: the measured intrinsic size can under-count the
            // real layout (e.g. `<hr>` reserves a trailing row, wide runs
            // wrap to more lines than the max-content measure assumed).
            // Grow the rect to fit the content instead of letting the last
            // row bleed into the border.
            let content_border = content_height
                .max(line_height_cells.max(1))
                .saturating_add(edges.padding.top)
                .saturating_add(edges.padding.bottom)
                .saturating_add(edges.border.top.thickness())
                .saturating_add(edges.border.bottom.thickness());
            height.max(content_border)
        }
        None => {
            let mut height = reserve_border_box(
                cursor,
                geom.outer_y,
                edges,
                content_height,
                line_height_cells.max(1),
            );
            // `min-height` / `max-height` clamp the border-box height
            // (converted per `box-sizing`). `min-height` can push the box
            // past its content; `max-height` caps it (content is clipped by
            // the paint phase).
            let inner_v = edges.padding.top
                + edges.padding.bottom
                + edges.border.top.thickness()
                + edges.border.bottom.thickness();
            let to_border = |h: usize| match sizing {
                BoxSizing::BorderBox => h,
                BoxSizing::ContentBox => h.saturating_add(inner_v),
            };
            // `height` fixes the border-box height (converted per
            // `box-sizing`), replacing the content-derived height: the box
            // keeps its intended frame even when the wrapped text / block
            // children are taller, and the paint phase clips the overflow.
            // On a narrow terminal the frame therefore wins over the text
            // instead of the box silently growing past its declared size.
            if let Some(h) = node
                .css_attr
                .get_height(None, child_context.font, child_context.viewport)
                .map(to_border)
            {
                height = h.max(1);
            }
            if let Some(m) = node
                .css_attr
                .get_min_height(None, child_context.font, child_context.viewport)
                .map(to_border)
            {
                height = height.max(m);
            }
            if let Some(m) = node
                .css_attr
                .get_max_height(None, child_context.font, child_context.viewport)
                .map(to_border)
            {
                height = height.min(m).max(1);
            }
            // `reserve_border_box` already advanced the cursor using the
            // pre-clamp height; re-position it so the next sibling starts
            // below the clamped border-box (otherwise the parent's content
            // area would stay short and the paint phase would clip this
            // box's extra rows).
            cursor.y = geom
                .outer_y
                .saturating_add(height)
                .saturating_add(edges.margin.bottom);
            // Leave the bottom margin for the next in-flow sibling to
            // collapse its top margin against.
            cursor.leave_bottom_margin(edges.margin.bottom);
            height
        }
    };

    // Spacing belongs to block flow. A flex formatting context has already
    // allocated the item's border-box, so adding block-flow spacing here
    // would move the item outside the rectangle chosen by flex.
    let rect_height = if preallocated_height.is_some() {
        outer_height
    } else {
        outer_height + opts.spacing_after
    };
    if preallocated_height.is_none() && opts.spacing_after > 0 {
        cursor.y = cursor.y.saturating_add(opts.spacing_after);
    }

    let effective_spacing_after = if preallocated_height.is_some() {
        0
    } else {
        opts.spacing_after
    };

    let paint_style = PaintStyle {
        box_edges: edges,
        runs: if wrapped_lines.is_empty() || wrapped_lines.iter().all(|l| l.is_empty()) {
            None
        } else {
            Some(wrapped_lines)
        },
        column_layout,
        font_size_cells: own_font_size_cells,
        line_height_cells,
        border_radius: node.css_attr.get_border_radius(
            containing.width,
            child_context.font,
            child_context.viewport,
        ),
        fill_bg,
        spacing_after: effective_spacing_after,
    };

    LayoutBox::new_normal(
        node_id,
        build_border_rect(geom, rect_height),
        paint_style,
        node.css_attr.outer_display(&node.node_type),
        children,
    )
}

/// Layer the browser UA default border (`opts.ua_border`) onto the resolved
/// CSS edges, mirroring the cascade where the UA stylesheet loses to any
/// author declaration — but at component granularity, like a browser:
/// `dialog { border-width: 2px }` keeps the UA `solid` style / default color
/// while only the width comes from the author.
///
/// Each side keeps a UA component (width / style / color) only while neither
/// the `border` shorthand, that side's `border-{side}` shorthand, nor the
/// universal `border-{component}` longhand is declared by the author.
fn merge_ua_border(node: &Node, edges: BoxEdges, ua_border: Option<BorderSides>) -> BoxEdges {
    let Some(ua) = ua_border else {
        return edges;
    };
    let css = &node.css_attr;
    let authored = |side: &str, component: &str| -> bool {
        css.is_set("border")
            || css.is_set(&format!("border-{side}"))
            || css.is_set(&format!("border-{component}"))
    };
    let merge = |side: &str, author: Border, ua_side: Border| Border {
        width: if authored(side, "width") {
            author.width
        } else {
            ua_side.width
        },
        style: if authored(side, "style") {
            author.style
        } else {
            ua_side.style
        },
        color: if authored(side, "color") {
            author.color
        } else {
            ua_side.color
        },
    };
    BoxEdges {
        border: BorderSides {
            top: merge("top", edges.border.top, ua.top),
            right: merge("right", edges.border.right, ua.right),
            bottom: merge("bottom", edges.border.bottom, ua.bottom),
            left: merge("left", edges.border.left, ua.left),
        },
        ..edges
    }
}

/// Layer the browser UA default margins (`opts.ua_margins`) onto the
/// resolved CSS edges.
///
/// Each side keeps its UA value only while neither the `margin` shorthand
/// nor that side's longhand is declared by the author — the same precedence
/// an author stylesheet has over the UA stylesheet in the browser cascade.
fn merge_ua_margins(node: &Node, mut edges: BoxEdges, ua_margins: Option<EdgeSizes>) -> BoxEdges {
    let Some(ua) = ua_margins else {
        return edges;
    };
    if !node.css_attr.is_set("margin") && !node.css_attr.is_set("margin-top") {
        edges.margin.top = ua.top;
    }
    if !node.css_attr.is_set("margin") && !node.css_attr.is_set("margin-right") {
        edges.margin.right = ua.right;
    }
    if !node.css_attr.is_set("margin") && !node.css_attr.is_set("margin-bottom") {
        edges.margin.bottom = ua.bottom;
    }
    if !node.css_attr.is_set("margin") && !node.css_attr.is_set("margin-left") {
        edges.margin.left = ua.left;
    }
    edges
}

/// One unit of content in the multicol stream: a wrapped inline line or a
/// block child. `height` spans from the unit's start down to the next unit's
/// start in the pass-1 stream, so inter-sibling margins survive
/// fragmentation.
struct ColumnFragment {
    height: usize,
    /// Index of the wrapped inline line, when this unit is a line fragment.
    line: Option<usize>,
    /// Index into the children vec, when this unit is a child fragment.
    child: Option<usize>,
}

/// Lay out a box's content into side-by-side columns (CSS Multi-column).
///
/// Pass 1 wraps the inline runs at the column width and lays the block
/// children out as one continuous `col_width`-wide stream. The stream is then
/// fragmented into `count` columns: `balance` (the default) targets
/// `ceil(total / count)` per column, `auto` targets the box's explicit
/// content height, and each unit flows into the next column when the current
/// one would overflow. Children are re-positioned to their column's
/// coordinates; detached (`absolute` / `fixed`) children keep their pass-1
/// rectangles and are left to the post-layout positioning pass. Returns the
/// per-column text table for the paint phase, the re-positioned children, and
/// the content height (the tallest column).
#[allow(clippy::too_many_arguments)]
fn layout_multicol(
    arena: &NodeArena,
    node_id: NodeId,
    all_runs: Vec<InlineRun>,
    geom: OuterGeometry,
    spec: &ColumnsResolved,
    content_top: usize,
    own_font_size_cells: usize,
    line_height_cells: usize,
    white_space: WhiteSpace,
    word_break: WordBreak,
    overflow_wrap: OverflowWrap,
    block_fg: Option<Color>,
    fixed_content_height: Option<usize>,
    child_context: LayoutContext,
) -> (Option<ColumnLayout>, Vec<LayoutBox>, usize) {
    let count = spec.count.max(2);
    let col_w = spec.col_width.max(1);
    let gap = spec.gap;
    let content_left = geom.content_constraint.x;

    // Pass 1: wrap the inline content to the column width, then lay the block
    // children out in a single `col_w`-wide stream starting below the inline
    // lines (same document order the single-column path uses).
    let wrapped_lines = if all_runs.is_empty() {
        Vec::new()
    } else {
        wrap_runs(
            &all_runs,
            col_w,
            own_font_size_cells,
            white_space,
            word_break,
            overflow_wrap,
        )
    };
    let line_rows = wrapped_lines.len().saturating_mul(line_height_cells.max(1));
    let block_children = visible_children(arena, node_id);
    let mut col_cursor = Cursor {
        y: content_top.saturating_add(line_rows),
        prev_bottom_margin: 0,
    };
    let mut children = layout_children(
        arena,
        &block_children,
        BoxConstraint {
            x: content_left,
            width: col_w,
        },
        &mut col_cursor,
        child_context,
    );

    // Build the fragment stream. Inline lines first (contiguous), then each
    // in-flow child (static / relative) using its pass-1 rect, so the height
    // spans margins and relative offsets down to the next unit.
    let mut fragments: Vec<ColumnFragment> = Vec::new();
    let mut stream = 0usize;
    for idx in 0..wrapped_lines.len() {
        fragments.push(ColumnFragment {
            height: line_height_cells.max(1),
            line: Some(idx),
            child: None,
        });
        stream = stream.saturating_add(line_height_cells.max(1));
    }
    for (idx, child) in children.iter().enumerate() {
        if !matches!(
            child.positioning.position,
            Position::Static | Position::Relative
        ) {
            continue;
        }
        let end = child
            .rect
            .y
            .saturating_add(child.rect.height)
            .saturating_sub(content_top);
        fragments.push(ColumnFragment {
            height: end.saturating_sub(stream),
            line: None,
            child: Some(idx),
        });
        stream = end;
    }
    let total = stream;

    // Distribute the fragments into columns. `target` is the height each
    // column is allowed before the next fragment moves on. For `balance`
    // (the default) it is the *smallest* column height that fits the content
    // into `count` columns — browsers balance by finding that height (a plain
    // `ceil(total / count)` target is wrong when fragments are uneven: three
    // 3-row blocks in 2 columns want 6/3, not 3/6). `auto` with an explicit
    // container height targets that height instead.
    let target = match (spec.fill, fixed_content_height) {
        (ColumnFill::Auto, Some(h)) => h.max(1),
        _ => balance_column_height(&fragments, count, total),
    };
    let mut col_of = vec![0usize; fragments.len()];
    let mut col_off = vec![0usize; fragments.len()];
    let mut col = 0usize;
    let mut col_h = 0usize;
    for (i, frag) in fragments.iter().enumerate() {
        if col_h > 0 && col_h.saturating_add(frag.height) > target && col < count.saturating_sub(1)
        {
            col += 1;
            col_h = 0;
        }
        col_of[i] = col;
        col_off[i] = col_h;
        col_h = col_h.saturating_add(frag.height);
    }

    // The container's content height is the tallest column.
    let mut col_heights = vec![0usize; count];
    for (i, frag) in fragments.iter().enumerate() {
        col_heights[col_of[i]] =
            col_heights[col_of[i]].max(col_off[i].saturating_add(frag.height));
    }
    let content_height = col_heights.iter().copied().max().unwrap_or(0);

    // Re-position the in-flow children into their columns. The column x is
    // `content_left + col * (col_w + gap)`; the column y keeps the fragment's
    // offset within its column (margins and relative offsets are already
    // folded into the fragment spans).
    for (i, frag) in fragments.iter().enumerate() {
        let Some(child_idx) = frag.child else {
            continue;
        };
        let child = &mut children[child_idx];
        let col_x = content_left.saturating_add(
            col_of[i].saturating_mul(col_w.saturating_add(gap)),
        );
        child.rect.x = col_x.saturating_add(child.rect.x.saturating_sub(content_left));
        child.rect.y = content_top.saturating_add(col_off[i]);
    }

    // Group the wrapped lines per column for the paint phase.
    let mut column_lines: Vec<Vec<Vec<InlineRun>>> = vec![Vec::new(); count];
    for (i, frag) in fragments.iter().enumerate() {
        if let Some(line_idx) = frag.line {
            column_lines[col_of[i]].push(wrapped_lines[line_idx].clone());
        }
    }

    let mut col_xs: Vec<usize> = Vec::with_capacity(count);
    for c in 0..count {
        col_xs.push(
            content_left.saturating_add(c.saturating_mul(col_w.saturating_add(gap))),
        );
    }

    // Column rules sit centred in each gap; a zero-width gap leaves no room.
    let mut rule_xs: Vec<usize> = Vec::new();
    let rule_visible = spec.rule.is_some_and(|r| r.style != BorderStyle::None) && gap > 0;
    if rule_visible {
        for c in 0..count.saturating_sub(1) {
            let boundary =
                content_left.saturating_add((c + 1).saturating_mul(col_w)).saturating_add(
                    c.saturating_mul(gap),
                );
            rule_xs.push(boundary.saturating_add(gap / 2));
        }
    }
    let rule_color = spec
        .rule
        .filter(|r| r.style != BorderStyle::None)
        .and_then(|r| r.color)
        .or(block_fg);

    let layout = ColumnLayout {
        lines: column_lines,
        col_x: col_xs,
        col_width: col_w,
        rule_x: rule_xs,
        rule_color,
        height: content_height,
    };
    (Some(layout), children, content_height)
}

/// Smallest column height that packs every fragment into `count` columns,
/// found by binary search over `[1, total]` — the browser column-balancing
/// algorithm. The feasibility test uses the same greedy per-fragment rule as
/// the distribution pass ([`fits_in_columns`]), so the height that "fits" is
/// exactly the height the distribution uses.
fn balance_column_height(fragments: &[ColumnFragment], count: usize, total: usize) -> usize {
    let mut lo = 1usize;
    let mut hi = total.max(1);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if fits_in_columns(fragments, count, mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// Whether the fragments pack into at most `count` columns when each column
/// is capped at `height`: a fragment moves to the next column only when the
/// current one already has content and would overflow; a fragment taller than
/// `height` overflows its column instead of breaking (matching browsers —
/// unbreakable content keeps flowing inside its column).
fn fits_in_columns(fragments: &[ColumnFragment], count: usize, height: usize) -> bool {
    let mut col = 0usize;
    let mut col_h = 0usize;
    for frag in fragments {
        if col_h > 0 && col_h.saturating_add(frag.height) > height {
            col += 1;
            if col >= count {
                return false;
            }
            col_h = 0;
        }
        col_h = col_h.saturating_add(frag.height);
    }
    true
}

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox, PaintStyle, ReplacedContent,
        inner_formatting_context::flow::FontContext,
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a normal-flow image box without a parent pre-allocation.
///
/// This compatibility wrapper keeps the original public five-argument API;
/// the layout registry uses [`layout_img_with_rect`] so flex can provide an
/// explicit item rectangle.
pub fn layout_img(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    layout_img_with_rect(arena, node_id, containing, cursor, font_ctx, None)
}

/// Build an `<img>` element, optionally honoring a rectangle allocated by a
/// parent formatting context.
pub fn layout_img_with_rect(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    _font_ctx: FontContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Img {
        return None;
    }

    let src = match node.get_src() {
        Some(s) => s.clone(),
        None => return None,
    };

    let (x, y, block_width, img_rows) = match pre_allocated_rect {
        Some(rect) => {
            let width = rect.width.max(1);
            let height = rect.height.max(1);
            (rect.x, rect.y, width, height)
        }
        None => {
            let max_w = node.get_width().unwrap_or(containing.width as u32 * 2);
            // `BoxConstraint` only exposes the containing block width, so the default
            // height is approximated as 4× that in pixels (i.e. a 2:1 portrait by
            // default once converted to cells). The picture always scrolls
            // vertically, so we never need to clamp to the terminal height.
            let max_h = node.get_height().unwrap_or(containing.width as u32 * 4);

            // Each terminal cell represents a 2×4 pixel block (see Img::_create_img).
            let img_cols = max_w.div_ceil(2).max(1) as usize;
            let img_rows = max_h.div_ceil(4).max(1) as usize;
            // Clamp to the containing block width so the rect and the rendered image
            // share the same cell count — painting past `rect.width` would overwrite
            // neighboring boxes.
            let block_width = img_cols.min(containing.width).max(1);
            let x = centered_left(containing, block_width);
            let y = cursor.allocate(img_rows);
            (x, y, block_width, img_rows)
        }
    };

    Some(LayoutBox::new_replaced(
        node_id,
        crate::layout::Rect::new(x, y, block_width, img_rows),
        ReplacedContent::Image {
            path: src,
            cols: block_width,
            rows: img_rows,
        },
        PaintStyle::default(),
    ))
}

fn centered_left(containing: BoxConstraint, block_width: usize) -> usize {
    containing.x + containing.width.saturating_sub(block_width) / 2
}

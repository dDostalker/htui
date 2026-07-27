
use crate::{
    layout::{BoxConstraint, Cursor, FontContext, LayoutBox},
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for an `<img>` element.
///
/// The rasterized dimensions are derived from the HTML `width` / `height`
/// attributes (each cell is a 2×4 pixel block, falling back to
/// `containing.width × 2` pixels wide and `containing.width × 4` pixels
/// tall when omitted). The path is taken from the `src` attribute verbatim
/// — the paint phase resolves it through `paint_image_at`.
pub fn layout_img(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    _font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Img {
        return None;
    }

    let src = match node.attributes.get_src() {
        Some(s) => s.clone(),
        None => return None,
    };

    let max_w = node
        .attributes
        .get_width()
        .unwrap_or(containing.width as u32 * 2);
    // `BoxConstraint` only exposes the containing block width, so the default
    // height is approximated as 4× that in pixels (i.e. a 2:1 portrait by
    // default once converted to cells). The picture always scrolls
    // vertically, so we never need to clamp to the terminal height.
    let max_h = node
        .attributes
        .get_height()
        .unwrap_or(containing.width as u32 * 4);

    // Each terminal cell represents a 2×4 pixel block (see Img::_create_img).
    let img_cols = max_w.div_ceil(2).max(1) as usize;
    let img_rows = max_h.div_ceil(4).max(1) as usize;
    // Clamp to the containing block width so the rect and the rendered image
    // share the same cell count — painting past `rect.width` would overwrite
    // neighboring boxes.
    let block_width = img_cols.min(containing.width);
    let x = centered_left(containing, block_width);
    let y = cursor.allocate(img_rows);

    Some(LayoutBox::new_image(
        node_id,
        crate::layout::Rect::new(x, y, block_width, img_rows),
        block_width,
        img_rows,
        src,
    ))
}

fn centered_left(containing: BoxConstraint, block_width: usize) -> usize {
    containing.x + containing.width.saturating_sub(block_width) / 2
}

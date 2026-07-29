use crate::{
    layout::{
        BoxConstraint, Cursor, FontContext, LayoutBox,
        tree::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for a `<p>` element.
///
/// Honors CSS `width` (default: fill containing block) — the resolved value is
/// the **border-box** width, including padding + border. Padding/border
/// shrink the content area passed to nested block children, and the inline
/// runs are painted on the content row (after `border.top` + `padding.top`).
/// `text-align` (inherited from ancestors via `inherit_css`) is computed
/// against the content area's width so `text-align: center` correctly
/// positions text within the inner content box rather than the full rect.
///
/// Margin / border / padding values are surfaced through `box_edges`; the
/// paint phase reads them to draw border glyphs and offset content.
pub fn layout_p(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::P {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        font_ctx,
    ))
}

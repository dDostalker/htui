
use crate::{
    layout::{
        BoxConstraint, Cursor, FontContext, LayoutBox, tree::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for an `<aside>` element.
///
/// `<aside>` is a block container whose rect fills its background even
/// without an explicit `background-color` so it reads as a solid stripe.
/// Padding / border are honoured: the inner containing block is shrunk, the
/// background fills the full border-box, and border glyphs are drawn when
/// `border-*` rules are present.
pub fn layout_aside(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Aside {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            fill_bg_default: true,
            chrome: true,
            ..Default::default()
        },
        font_ctx,
    ))
}

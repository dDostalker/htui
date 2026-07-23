use anyhow::Result;

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox,
        tree::{centered_left, layout_children, resolve_block_width},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for `<h1>`…`<h5>`.
///
/// Renders a `#`-marker prefix equal to the heading level (`# Heading`,
/// `## Sub`, …) so the paint phase includes it in `text-align` width
/// calculations, matching the previous behaviour.
pub fn layout_hn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>> {
    let node = &arena[node_id];
    let level = match node.node_type {
        NodeType::H1 => 1,
        NodeType::H2 => 2,
        NodeType::H3 => 3,
        NodeType::H4 => 4,
        NodeType::H5 => 5,
        _ => return Ok(None),
    };

    let block_width =
        resolve_block_width(node.css_attr.get_width(containing.width), containing.width);
    let x = centered_left(containing, block_width);
    let y = cursor.allocate(1);

    let prefix = "#".repeat(level);
    let body = collect_heading_text(arena, node);
    let combined = if body.is_empty() {
        body
    } else {
        format!("{prefix} {body}")
    };
    let text = if combined.is_empty() {
        None
    } else {
        Some(combined)
    };

    let child_containing = BoxConstraint {
        x,
        width: block_width,
    };
    let child_cursor = &mut Cursor { y: cursor.peek() };
    let block_children = node.children.clone();
    let children = layout_children(arena, &block_children, child_containing, child_cursor)?;
    cursor.y = child_cursor.y;

    Ok(Some(LayoutBox::new_block(
        node_id,
        crate::layout::Rect::new(x, y, block_width, 1),
        false,
        0,
        text,
        children,
    )))
}

/// Concatenate direct text descendants into one inline run. Mirrors
/// `tree::collect_direct_text` but kept local to the heading module so the
/// prefix logic above stays self-contained.
fn collect_heading_text(arena: &NodeArena, node: &crate::tdom::node::Node) -> String {
    use crate::tdom::node::NodeType as NT;
    let mut out = String::new();
    for child_id in node.child_ids() {
        let child = &arena[child_id];
        match &child.node_type {
            NT::Text(s) => out.push_str(s),
            NT::Span => out.push_str(&collect_heading_text(arena, child)),
            _ => {}
        }
    }
    out
}

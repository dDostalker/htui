//! Block-level layout for the `<a>` (anchor / hyperlink) element.
//!
//! An anchor with an explicit block-level display participates in block flow
//! (e.g. `<aside><a style="display: block">Dashboard</a></aside>`). An anchor
//! without that declaration remains in the parent's inline flow; the inline
//! path is handled by [`crate::layout::layoutbox::collect_runs`]. Flex items
//! are also handled here when the flex formatter supplies a pre-allocated
//! rectangle, because flex items are blockified by their container.
//!
//! Both paths apply the same browser-style user-agent defaults — blue
//! foreground and an underlined text-decoration — whenever neither an author
//! rule nor an inherited value has declared the matching property. The block
//! path injects these defaults into a cloned [`Node`] before delegating to
//! [`layout_block`]; the inline path applies the same defaults through
//! `collect_runs`' local `fg` / `style` variables without touching shared state.

use crate::{
    html::css_parse::CssAttr,
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext,
        inner_formatting_context::flow::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// Build a [`LayoutBox`] for an `<a>` element when it is a direct block-level
/// child of another block container.
///
/// Honors CSS `width` / `padding` / `border` / `margin` like any other block;
/// the only element-specific behaviour is the user-agent default styling
/// applied by [`apply_a_ua_defaults`]. Returns `None` for any non-`<a>` node
/// so the registry falls through to the next handler.
pub fn layout_a(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::A {
        return None;
    }
    // A normal anchor is inline by default. When flex has already allocated
    // its item rectangle, however, it must be laid out as a blockified item.
    if pre_allocated_rect.is_none() && !node.css_attr.display().is_some_and(|d| d.is_block_level())
    {
        return None;
    }

    let mut node = node.clone();
    apply_a_ua_defaults(&mut node.css_attr);

    Some(layout_block(
        arena,
        &node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        context,
        pre_allocated_rect,
    ))
}

/// Inject the browser-style `<a>` defaults into `css_attr` when the cascade
/// has not already recorded a value.
///
/// `is_set` returns `true` for any value the cascade has populated — explicit
/// author rules *and* inherited values from ancestors — so the defaults never
/// override something the author declared. The mutation is idempotent: a
/// second pass sees `is_set == true` and skips the insert, so re-running the
/// layout (e.g. after an interactive refresh) does not stomp on anything.
fn apply_a_ua_defaults(css_attr: &mut CssAttr) {
    for &(prop, value) in NodeType::A.inline_ua_declarations() {
        if !css_attr.is_set(prop) {
            css_attr.0.insert(prop.to_string(), value.to_string());
        }
    }
}

//! Standard block elements driven by a `NodeType → LayoutBlockOptions` table.
//!
//! Most block-level elements share the exact same block pipeline
//! ([`layout_block`]) and differ only in the options they hand it. Instead of
//! one copy-pasted adapter per tag, a single [`layout_standard_block`] looks
//! the node type up in [`STANDARD_BLOCK_ELEMENTS`]. Elements with dynamic
//! options (headings, lists, anchors with UA defaults) keep their own
//! adapters in this module's siblings.

use crate::{
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext,
        inner_formatting_context::flow::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{NodeArena, NodeId, NodeType},
};

/// One entry in the standard-block table: a node type plus the options the
/// shared block pipeline receives.
struct BlockElementConfig {
    node_type: NodeType,
    opts: fn() -> LayoutBlockOptions,
}

fn default_opts() -> LayoutBlockOptions {
    LayoutBlockOptions::default()
}

/// `<aside>` reads as a solid stripe even without an explicit background.
fn aside_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        fill_bg_default: true,
        chrome: true,
        ..Default::default()
    }
}

/// `<header>` / `<footer>` chrome bands with breathing room below.
fn header_footer_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        fill_bg_default: true,
        chrome: true,
        spacing_after: 2,
        ..Default::default()
    }
}

/// `<address>` keeps the browser user-agent `font-style: italic` default.
fn address_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        ua_italic: true,
        ..Default::default()
    }
}

/// Plain block elements handled by [`layout_standard_block`].
static STANDARD_BLOCK_ELEMENTS: &[BlockElementConfig] = &[
    BlockElementConfig {
        node_type: NodeType::Div,
        opts: default_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Body,
        opts: default_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Html,
        opts: default_opts,
    },
    BlockElementConfig {
        node_type: NodeType::P,
        opts: default_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Aside,
        opts: aside_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Header,
        opts: header_footer_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Footer,
        opts: header_footer_opts,
    },
    BlockElementConfig {
        node_type: NodeType::HGroup,
        opts: default_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Address,
        opts: address_opts,
    },
];

/// Build a [`LayoutBox`] for any element in [`STANDARD_BLOCK_ELEMENTS`].
///
/// Returns `None` for every other node type so the layout registry falls
/// through to the specialised adapters.
pub fn layout_standard_block(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    let config = STANDARD_BLOCK_ELEMENTS
        .iter()
        .find(|config| config.node_type == node.node_type)?;
    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        (config.opts)(),
        context,
        pre_allocated_rect,
    ))
}

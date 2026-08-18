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
        Border, BorderSides, BorderStyle, BoxConstraint, Cursor, EdgeSizes, LayoutBox,
        LayoutContext,
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

/// Browser UA default `margin-block: 1em` for `<p>` — one blank row above
/// and below the paragraph, adapted to terminal cells.
fn p_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        ua_margins: Some(EdgeSizes::new(1, 0, 1, 0)),
        ..Default::default()
    }
}

/// `<aside>` is a plain block — no browser UA default background or chrome.
fn aside_opts() -> LayoutBlockOptions {
    LayoutBlockOptions::default()
}

/// `<header>` / `<footer>` are plain blocks — no browser UA default chrome
/// or trailing blank rows.
fn header_footer_opts() -> LayoutBlockOptions {
    LayoutBlockOptions::default()
}

/// `<address>` keeps the browser user-agent `font-style: italic` default.
fn address_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        ua_italic: true,
        ..Default::default()
    }
}

/// Browser UA default margins for `<blockquote>` (`margin-block: 1em`,
/// `margin-inline: 40px`), adapted to terminal cells: 1 blank row above /
/// below and 4 columns on each side.
const BLOCKQUOTE_MARGIN_BLOCK: usize = 1;
const BLOCKQUOTE_MARGIN_INLINE: usize = 4;

/// `<blockquote>` keeps the browser user-agent margin defaults: one blank
/// row above/below and a four-column indent on each side.
fn blockquote_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        ua_margins: Some(EdgeSizes::new(
            BLOCKQUOTE_MARGIN_BLOCK,
            BLOCKQUOTE_MARGIN_INLINE,
            BLOCKQUOTE_MARGIN_BLOCK,
            BLOCKQUOTE_MARGIN_INLINE,
        )),
        ..Default::default()
    }
}

/// Browser UA default `margin-block: 1em` for `<dl>` — one blank row
/// above/below the description list.
const DL_MARGIN_BLOCK: usize = 1;

/// Browser UA default `margin-inline-start: 40px` for `<dd>`, adapted to
/// 4 terminal columns.
const DD_MARGIN_INLINE: usize = 4;

/// `<dl>` keeps the browser user-agent `margin-block: 1em` default: one
/// blank row above/below the description list.
fn dl_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        ua_margins: Some(EdgeSizes::new(DL_MARGIN_BLOCK, 0, DL_MARGIN_BLOCK, 0)),
        ..Default::default()
    }
}

/// `<dd>` keeps the browser user-agent `margin-inline-start: 40px` default:
/// a four-column indent that separates it from its `<dt>` term.
fn dd_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        ua_margins: Some(EdgeSizes::new(0, 0, 0, DD_MARGIN_INLINE)),
        ..Default::default()
    }
}

/// `<dialog open>` keeps the browser UA default border (`border: solid`,
/// one cell wide) unless the author declares a border property — the framed
/// look that separates a dialog from a plain block.
fn dialog_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        ua_border: Some(BorderSides::all(Border::new(1, BorderStyle::Solid, None))),
        ..Default::default()
    }
}

/// `<fieldset>` keeps the browser UA default border plus one blank row of
/// UA margin above / below, framing the control group.
fn fieldset_opts() -> LayoutBlockOptions {
    LayoutBlockOptions {
        ua_border: Some(BorderSides::all(Border::new(1, BorderStyle::Solid, None))),
        ua_margins: Some(EdgeSizes::new(1, 0, 1, 0)),
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
        opts: p_opts,
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
    BlockElementConfig {
        node_type: NodeType::BlockQuote,
        opts: blockquote_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Dl,
        opts: dl_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Dt,
        opts: default_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Dd,
        opts: dd_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Search,
        opts: default_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Dialog,
        opts: dialog_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Fieldset,
        opts: fieldset_opts,
    },
    BlockElementConfig {
        node_type: NodeType::Legend,
        opts: default_opts,
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

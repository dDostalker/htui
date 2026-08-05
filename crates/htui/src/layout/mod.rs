//! Layout pipeline.
//!
//! ```text
//! DOM → LayoutBox tree → Block grid
//! ```
//!
//! The two-phase pipeline is split as follows:
//!
//! - [`layoutbox`] defines the [`LayoutBox`] data model and its three-axis
//!   classification ([`BoxKind`], [`OuterDisplay`], [`FormattingContext`]).
//! - [`layoutbox::inner_formatting_context::flow`], [`flex`], and [`grid`]
//!   implement the three formatting contexts that build layout trees from a
//!   DOM. Each lives under
//!   [`layoutbox::inner_formatting_context`] and shares the same
//!   [`LayoutBox`] tree shape.
//! - [`paint`] walks a finished `Vec<LayoutBox>` and writes
//!   [`Block`](crate::tui::graph_api::Block) cells into the terminal canvas.
//! - [`layout`] (this file) glues everything together: [`DEFAULT_LAYOUT`] is
//!   the registry of layout functions consulted by the flow orchestrator.

use crate::{
    layout::layout::TermMap,
    tdom::node::{NodeArena, NodeId},
};

pub mod layout;
pub mod layoutbox;
pub mod paint;

pub use crate::html::box_edges::{Border, BorderSides, BorderStyle, BoxEdges, EdgeSizes};
pub use crate::html::css_parse::{FontContext, Viewport};
pub use layoutbox::inner_formatting_context;
pub use layoutbox::inner_formatting_context::{
    flex::layout_flex_dispatch,
    flow::layout_tree,
};
pub use layoutbox::{
    BoxKind, Cursor, FormattingContext, GeneratedBox, GeneratedBoxKind, InlineRun, LayoutBox,
    OuterDisplay, PaintStyle, Rect, ReplacedContent, generate_box_tree, generate_node,
    layout_generated_box, layout_generated_tree,
};

/// Context propagated by the layout phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutContext {
    pub font: FontContext,
    pub viewport: Viewport,
}

impl LayoutContext {
    pub fn child(self, child_font_size_cells: usize) -> Self {
        Self {
            font: self.font.child(child_font_size_cells),
            viewport: self.viewport,
        }
    }
}

///
/// A child element resolves its own width / horizontal position relative to
/// its containing block, mirroring CSS containing-block semantics.
#[derive(Debug, Clone, Copy)]
pub struct BoxConstraint {
    pub x: usize,
    pub width: usize,
}

impl BoxConstraint {
    /// Creates a root constraint spanning the terminal width.
    pub fn root(term_map: &TermMap) -> Self {
        Self {
            x: 0,
            width: term_map.columns as usize,
        }
    }
}

/// Layout function signature for the two-phase pipeline.
///
/// Each registered function inspects `node_id`; returns `None` to skip
/// (passes through to the next entry), or `Some(LayoutBox)` to claim the node.
/// The builder in
/// [`layoutbox::inner_formatting_context::flow::layout_tree`] walks the
/// registry in order and stops at the first claimer. `font_ctx` carries the
/// parent's effective font-size (and the root font-size) so `em` / `rem`
/// lengths resolve against the active font-size context.
///
/// `pre_allocated_rect` lets the caller (currently only the flex layout) hand
/// the layout function a fixed outer rect, instead of computing one from the
/// shared `cursor`. The layout function MUST honor it when present — flex
/// items have their rectangle determined by the flex algorithm, not by the
/// block flow. `None` means "allocate the rect from the cursor yourself".
/// This mirrors the way browser layout engines let a formatting context
/// (Block Formatting Context, Flex Formatting Context, …) determine the
/// outer rect of a child and then call into the child's layout function with
/// that rect — the child does not recompute it.
pub type LayoutFn = fn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox>;

/// Layout function registry.
///
/// Each element type that needs custom layout declares a function here. The
/// dispatcher iterates this list and uses the first non-`None` result.
///
/// The geometry dispatcher handles replaced, flex, and table boxes before this
/// registry is reached. This registry therefore contains only normal-flow
/// element adapters retained for behavior and public-API compatibility.
///
/// Plain block elements (`<div>` / `<p>` / `<aside>` / …) are folded into
/// [`layout_standard_block`](inner_formatting_context::flow::layout_standard_block)
/// and its `NodeType → LayoutBlockOptions` table; the remaining entries are
/// the dynamically-optioned adapters (headings, lists, anchors) and the
/// table-element pass-throughs kept for API compatibility.
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[
    inner_formatting_context::flow::layout_standard_block,
    inner_formatting_context::flow::layout_hn,
    inner_formatting_context::flow::layout_title,
    inner_formatting_context::flow::layout_meta,
    inner_formatting_context::flow::layout_a,
    inner_formatting_context::flow::layout_li,
    inner_formatting_context::flow::layout_ol,
    inner_formatting_context::flow::layout_ul,
    inner_formatting_context::flow::layout_row_group,
    inner_formatting_context::flow::layout_td,
    inner_formatting_context::flow::layout_th,
    inner_formatting_context::flow::layout_tr,
    inner_formatting_context::flow::layout_generic,
];

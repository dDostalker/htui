use anyhow::Result;

use crate::{
    layout::layout::TermMap,
    tdom::node::{NodeArena, NodeId},
};

pub mod box_tree;
pub mod layout;
pub mod layout_fn;
pub mod paint;
pub mod tree;

pub use box_tree::{BoxKind, Cursor, InlineRun, LayoutBox, Rect};

/// The containing block available to a layout function.
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

    /// Resolves a child box centered inside this containing block.
    pub fn centered(self, width: usize) -> Self {
        let width = width.min(self.width);
        Self {
            x: self.x + self.width.saturating_sub(width) / 2,
            width,
        }
    }
}

/// Formatting context used to propagate containing-block constraints.
///
/// Currently a thin wrapper around [`BoxConstraint`]; kept as a separate type
/// so the registry signature can grow (e.g., direction, writing-mode) without
/// breaking every `LayoutFn`.
#[derive(Debug, Clone, Copy)]
pub struct FormattingContext {
    pub containing: BoxConstraint,
}

impl FormattingContext {
    pub fn root(term_map: &TermMap) -> Self {
        Self {
            containing: BoxConstraint::root(term_map),
        }
    }

    pub fn child(self, containing: BoxConstraint) -> Self {
        Self { containing }
    }
}

/// Layout function signature for the two-phase pipeline.
///
/// Each registered function inspects `node_id`; returns `Ok(None)` to skip
/// (passes through to the next entry), or `Ok(Some(LayoutBox))` to claim the
/// node. The builder in [`tree::layout_tree`] walks the registry in order and
/// stops at the first claimer.
pub type LayoutFn = fn(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
) -> Result<Option<LayoutBox>>;

/// Layout function registry.
///
/// Each element type that needs custom layout declares a function here. The
/// dispatcher iterates this list and uses the first non-`None` result.
pub static DEFAULT_LAYOUT: &[LayoutFn] = &[
    layout_fn::p::layout_p,
    layout_fn::head::layout_hn,
    layout_fn::img::layout_img,
    layout_fn::div::layout_div,
    layout_fn::header_footer::layout_header,
    layout_fn::header_footer::layout_footer,
    layout_fn::aside::layout_aside,
    layout_fn::list::layout_li,
    layout_fn::list::layout_ol,
    layout_fn::list::layout_ul,
];

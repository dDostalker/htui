//! Layout box tree — the intermediate representation between the DOM and the
//! terminal canvas.
//!
//! A [`LayoutBox`] describes a single rectangular region on the virtual canvas
//! (one block-level element or one inline run), its computed style, and its
//! nested children. The two-phase pipeline is:
//!
//! ```text
//! DOM → LayoutBox tree → Block grid
//! ```
//!
//! - [`layout_tree`](crate::layout::layoutbox::inner_formatting_context::flow::layout_tree) builds the tree and
//!   assigns rectangles from containing-block constraints.
//! - [`paint_layout_tree`](crate::layout::paint::paint_layout_tree) walks the
//!   tree and writes [`Block`](crate::graphics::Block) cells into the
//!   terminal canvas.
//!
//! ## Architecture
//!
//! [`LayoutBox`] follows the browser CSS display model: each box carries three
//! independent properties that together describe how it renders.
//!
//! 1. `kind` — the box's *content shape*:
//!    [`BoxKind::Normal`] (a regular block container), [`BoxKind::Replaced`]
//!    (a leaf whose content is supplied by something other than the box
//!    itself — currently `<img>`), or [`BoxKind::Anonymous`] (a synthetic
//!    wrapper the layout engine inserts when text or inline elements need to
//!    be grouped into a single block).
//!
//! 2. `outer_display` — how this box *participates in its parent's*
//!    formatting context ([`OuterDisplay::BlockLevel`] vs
//!    [`OuterDisplay::Inline`], with each variant carrying a
//!    `container` / `block` flag that distinguishes a block container
//!    from a plain block-level box, and `inline-block` from `inline`).
//!
//! 3. `inner_formatting_context` — how this box *lays out its own children*:
//!    [`FormattingContext::Flow`] (default block / inline flow),
//!    [`FormattingContext::Flex`] (CSS flexbox), [`FormattingContext::Table`]
//!    (table formatting context), or [`FormattingContext::Grid`]
//!    (reserved for future grid support).
//!
//! The three axes are independent: a flex container is
//! `BoxKind::Normal + OuterDisplay::BlockLevel { container: true } + FormattingContext::Flex`;
//! an `<img>` is
//! `BoxKind::Replaced + OuterDisplay::Inline { block: true } + FormattingContext::Flow`
//! (its UA `display: inline-block` is what makes it a block-level
//! replaced inline-block box).
//! Paint-only data (padding / border / margin / inline runs / background-fill
//! flags / font-size) is collected in [`PaintStyle`] so [`LayoutBox`]
//! itself only carries layout-classification fields. This keeps the layout
//! tree small and makes the boundary between layout and paint explicit.
//!
//! ## Module layout
//!
//! ```text
//! layoutbox/
//! ├── mod.rs              ← LayoutBox + re-exports (this file)
//! ├── rect.rs             ← Rect
//! ├── paint_style.rs      ← PaintStyle  (computed_style / paint_style)
//! ├── inline_run.rs       ← InlineRun   (used by PaintStyle.runs)
//! ├── cursor.rs           ← Cursor      (block-flow row allocator)
//! ├── inline.rs           ← collect_runs (shared by block + flex layouts)
//! ├── wrap.rs             ← wrap_runs / wrapped_size (Text Level 3 line-breaker)
//! ├── inline.rs           ← collect_runs (shared by block + flex layouts)
//! ├── box_kind/           ← BoxKind + ReplacedContent
//! ├── outer_display/      ← OuterDisplay
//! ├── generation/         ← generate_box_tree / GeneratedBox / GeneratedBoxKind
//! ├── geometry/           ← layout_generated_tree / dispatch by classification
//! └── inner_formatting_context/  ← FormattingContext + flow / flex / grid
//!                                  implementations
//! ├── wrap.rs             ← wrap_runs (CSS Text Level 3 line-breaker)
//! ```

mod box_kind;
mod cursor;
pub mod generation;
pub mod geometry;
mod inline;
mod inline_run;
mod outer_display;
mod paint_style;
mod position;
mod rect;
mod wrap;

pub mod inner_formatting_context;

pub use box_kind::{BoxKind, ReplacedContent};
pub use cursor::Cursor;
pub use generation::{
    GeneratedBox, GeneratedBoxKind, generate_box_tree, generate_node, visible_children,
};
pub use geometry::{layout_generated_box, layout_generated_tree};
pub use inline::collect_runs;
pub(crate) use inline::{q_quote_pair, text_node_run};
pub use inline_run::InlineRun;
pub use inner_formatting_context::FormattingContext;
pub use outer_display::OuterDisplay;
pub use paint_style::{ColumnLayout, PaintStyle};
pub use position::{
    apply_relative_offset, pin_fixed_boxes, pin_sticky_boxes, resolve_positioned_rects,
};
pub use rect::Rect;
pub use wrap::{is_cjk_break_opportunity, wrap_runs, wrapped_size};

use crate::html::css_parse::Positioning;
use crate::tdom::node::NodeId;

/// One rectangle in the layout tree.
///
/// Each `LayoutBox` carries:
///
/// - `rect` — the **border-box** rectangle painted by the box. Includes
///   border, padding and the content area.
/// - `paint_style` — every paint-only property (padding / border / margin /
///   inline runs / font-size / background-fill flags).
/// - `kind` — the box's content shape ([`BoxKind`]).
/// - `outer_display` — how the box participates in its parent's flow.
/// - `inner_formatting_context` — how the box arranges its own children.
/// - `positioning` — resolved `position` / insets / `z-index`, driving the
///   post-layout positioning pass and paint stacking order.
/// - `children` — nested [`LayoutBox`]es laid out below this box.
///
/// The classification fields (`kind`, `outer_display`,
/// `inner_formatting_context`) are populated by the layout function that
/// built the box; the paint phase reads them only to choose between block /
/// replaced / anonymous handling.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub node_id: NodeId,
    pub rect: Rect,
    pub paint_style: PaintStyle,
    pub kind: BoxKind,
    /// How this box participates in its parent's formatting context.
    pub outer_display: OuterDisplay,
    /// How this box arranges its own children.
    pub inner_formatting_context: FormattingContext,
    pub positioning: Positioning,
    /// Normal-flow document row of a `position: sticky` box, captured on the
    /// first pin (right after layout, before the rect moves). The geometry
    /// cache holds already-pinned boxes across scrolls, so later pins reuse
    /// this instead of re-reading the moved rect. `None` for every other box.
    pub sticky_flow_y: Option<usize>,
    pub children: Vec<LayoutBox>,
}

impl LayoutBox {
    /// Build a [`BoxKind::Normal`] block container with the given
    /// [`PaintStyle`], `outer_display`, and children. The inner formatting
    /// context defaults to [`FormattingContext::Flow`].
    ///
    /// Use [`Self::new_normal_with_context`] when the box establishes a non-default
    /// inner formatting context (currently only flex).
    pub fn new_normal(
        node_id: NodeId,
        rect: Rect,
        paint_style: PaintStyle,
        outer_display: OuterDisplay,
        children: Vec<LayoutBox>,
    ) -> Self {
        Self::new_normal_with_context(
            node_id,
            rect,
            paint_style,
            outer_display,
            FormattingContext::Flow,
            children,
        )
    }

    /// Same as [`Self::new_normal`], but with an explicit inner formatting context.
    /// Use this for flex containers (which establish a
    /// [`FormattingContext::Flex`] for their children) or future grid
    /// containers.
    pub fn new_normal_with_context(
        node_id: NodeId,
        rect: Rect,
        paint_style: PaintStyle,
        outer_display: OuterDisplay,
        inner_formatting_context: FormattingContext,
        children: Vec<LayoutBox>,
    ) -> Self {
        Self {
            node_id,
            rect,
            paint_style,
            kind: BoxKind::Normal,
            outer_display,
            inner_formatting_context,
            positioning: Positioning::default(),
            sticky_flow_y: None,
            children,
        }
    }

    /// Convenience constructor for a flex container.
    ///
    /// A flex container paints as a block container (with its own background,
    /// border, padding and inline runs) but arranges its direct children by
    /// the flex algorithm. Use [`Self::new_normal_with_context`] with
    /// [`FormattingContext::Flex`] directly if more flexibility is needed.
    pub fn new_flex_container(
        node_id: NodeId,
        rect: Rect,
        paint_style: PaintStyle,
        outer_display: OuterDisplay,
        children: Vec<LayoutBox>,
    ) -> Self {
        Self::new_normal_with_context(
            node_id,
            rect,
            paint_style,
            outer_display,
            FormattingContext::Flex,
            children,
        )
    }

    /// Build a [`BoxKind::Replaced`] box.
    ///
    /// Replaced elements are leaves whose content is supplied by something
    /// other than the box's own subtree (e.g. `<img>`). The `paint_style` is
    /// typically minimal (no padding / border / margin / inline runs) because
    /// the replaced content fills the rect directly.
    pub fn new_replaced(
        node_id: NodeId,
        rect: Rect,
        content: ReplacedContent,
        paint_style: PaintStyle,
    ) -> Self {
        Self {
            node_id,
            rect,
            paint_style,
            kind: BoxKind::Replaced(content),
            outer_display: OuterDisplay::BlockLevel { container: false },
            inner_formatting_context: FormattingContext::Flow,
            positioning: Positioning::default(),
            sticky_flow_y: None,
            children: Vec::new(),
        }
    }

    /// Build a [`BoxKind::Anonymous`] wrapper.
    ///
    /// Anonymous boxes are produced by the layout engine itself when an
    /// inline formatting context needs to be wrapped for a block-level
    /// parent, or when raw text inside a flex container is promoted into an
    /// anonymous flex item. The current engine does not emit anonymous boxes
    /// yet; this constructor exists so future implementations have a typed
    /// entry point.
    pub fn new_anonymous(
        node_id: NodeId,
        rect: Rect,
        paint_style: PaintStyle,
        children: Vec<LayoutBox>,
    ) -> Self {
        Self {
            node_id,
            rect,
            paint_style,
            kind: BoxKind::Anonymous,
            outer_display: OuterDisplay::BlockLevel { container: true },
            inner_formatting_context: FormattingContext::Flow,
            positioning: Positioning::default(),
            sticky_flow_y: None,
            children,
        }
    }
}

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
//! - [`layout_tree`](crate::layout::block::layout_tree) builds the tree and
//!   assigns rectangles from containing-block constraints.
//! - [`paint_layout_tree`](crate::layout::paint::paint_layout_tree) walks the
//!   tree and writes [`Block`](crate::tui::graph_api::Block) cells into the
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
//!    formatting context ([`OuterDisplay::Block`] vs [`OuterDisplay::Inline`]).
//!
//! 3. `inner_formatting_context` — how this box *lays out its own children*:
//!    [`FormattingContext::Flow`] (default block / inline flow),
//!    [`FormattingContext::Flex`] (CSS flexbox), or [`FormattingContext::Grid`]
//!    (reserved for future grid support).
//!
//! The three axes are independent: a flex container is
//! `BoxKind::Normal + OuterDisplay::Block + FormattingContext::Flex`; an
//! `<img>` is `BoxKind::Replaced + OuterDisplay::Block + FormattingContext::Flow`.
//! Paint-only data (padding / border / margin / inline runs / background-fill
//! flags / chrome / font-size) is collected in [`PaintStyle`] so [`LayoutBox`]
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
//! ├── box_kind/           ← BoxKind + ReplacedContent (per-variant helpers)
//! ├── outer_display/      ← OuterDisplay (per-variant helpers)
//! └── inner_formatting_context/  ← FormattingContext (per-variant helpers
//!                                  + flow / flex / grid implementations)
//! ```

mod box_kind;
mod cursor;
mod inline;
mod inline_run;
mod outer_display;
mod paint_style;
mod rect;

pub mod inner_formatting_context;

pub use box_kind::{BoxKind, ReplacedContent};
pub use cursor::Cursor;
pub use inline::collect_runs;
pub use inline_run::InlineRun;
pub use inner_formatting_context::FormattingContext;
pub use outer_display::OuterDisplay;
pub use paint_style::PaintStyle;
pub use rect::Rect;

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
    pub children: Vec<LayoutBox>,
}

impl LayoutBox {
    /// Build a [`BoxKind::Normal`] block container with the given
    /// [`PaintStyle`], `outer_display`, and children. The inner formatting
    /// context defaults to [`FormattingContext::Flow`].
    ///
    /// Use [`new_normal_with_context`] when the box establishes a non-default
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

    /// Same as [`new_normal`], but with an explicit inner formatting context.
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
            outer_display: OuterDisplay::Block,
            inner_formatting_context: FormattingContext::Flow,
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
            outer_display: OuterDisplay::Block,
            inner_formatting_context: FormattingContext::Flow,
            children,
        }
    }
}
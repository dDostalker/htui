//! Box content-shape classification.
//!
//! [`BoxKind`] distinguishes a box's *content shape* — what it carries —
//! independently of the box's outer / inner display roles. A flex container
//! is still [`BoxKind::Normal`]; only its
//! [`crate::layout::LayoutBox::inner_formatting_context`] switches to
//! [`crate::layout::FormattingContext::Flex`].
//!
//! ## Variants
//!
//! - [`BoxKind::Normal`] — Regular block container.
//! - [`BoxKind::Replaced`] — Leaf whose content is supplied outside the DOM
//!   (an `<img>` rasterisation today; room for `<video>` / `<input>` later).
//! - [`BoxKind::Anonymous`] — Synthetic wrapper emitted by the layout engine
//!   itself.

/// Content payload of a [`BoxKind::Replaced`] box.
///
/// Browsers treat `<img>` as a *replaced element*: the box's content is
/// supplied by an external source (the image file) rather than by the box's
/// own subtree. We model that as a dedicated [`ReplacedContent`] variant so
/// future replaced elements (`<video>`, `<input>`, ...) have a natural home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplacedContent {
    /// `<img>` rasterized into a fixed-size grid of `cols` × `rows` cells.
    Image {
        path: String,
        cols: usize,
        rows: usize,
    },
    /// `<input>` — a single-line text field.
    ///
    /// The visible value / cursor / focus state live in
    /// [`TermLayout::input_states`](crate::tui::graph_api::TermLayout::input_states),
    /// keyed by node id; the box only marks this as an input leaf so the
    /// paint phase knows to draw a field.
    Input,
    /// `<select>` — a single-line dropdown field. The options are read from
    /// the node's `<option>` children (see `select_options`); the currently
    /// selected value lives in the `value` attribute.
    Select,
}

/// Distinguishes the rendering shape of a layout box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoxKind {
    /// Regular block container — has its own inline runs and a tree of nested
    /// block children. The vast majority of boxes (every block element that
    /// is not a replaced leaf) fall into this category.
    Normal,
    /// Replaced element: leaf whose content is not in the DOM. Currently only
    /// `<img>`. Carries its own [`ReplacedContent`] payload.
    Replaced(ReplacedContent),
    /// Anonymous wrapper produced by the layout engine itself, e.g. to
    /// surround text children for a flex container or to hold an inline
    /// formatting context. The current engine doesn't emit these yet, but
    /// the variant exists so the type stays exhaustive.
    Anonymous,
}
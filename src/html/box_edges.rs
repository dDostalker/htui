//! CSS box-model edge values: padding, border, margin.
//!
//! Each edge of a [`BoxEdges`] carries an [`EdgeSizes`] tuple (top, right,
//! bottom, left). Together with [`Rect`](crate::layout::box_tree::Rect) they
//! fully describe the space taken by one block-level element:
//!
//! ```text
//!    margin-top
//!   ┌───────────────────────────────┐
//!   │         border-top            │
//!   │ ┌───────────────────────────┐ │
//!   │ │       padding-top         │ │
//!   │ │ ┌───────────────────────┐ │ │
//!   │ │ │       content         │ │ │
//!   │ │ └───────────────────────┘ │ │
//!   │ │       padding-bottom      │ │
//!   │ └───────────────────────────┘ │
//!   │         border-bottom         │
//!   └───────────────────────────────┘
//! ```
//!
//! The layout function stores the full *border-box* in
//! [`Rect`](crate::layout::box_tree::Rect) and the content-box (rect shrunk by
//! padding + border) is the inner [`BoxConstraint`](crate::layout::BoxConstraint)
//! passed to nested children. Margins live *outside* the rect — they offset the
//! rect within its containing block and reserve rows between siblings.
//!
//! This module lives under `html` because the edge values are a parsed-CSS
//! concept: `css_parse::CssAttr` hands them back to `layout` as resolved
//! values. Putting them under `html` keeps the architecture direction
//! `html → tdom → layout → tui` instead of inverting it.

use crossterm::style::Color;

/// Per-side lengths in cells. All four fields are non-negative.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeSizes {
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
    pub left: usize,
}

impl EdgeSizes {
    pub const fn new(top: usize, right: usize, bottom: usize, left: usize) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Horizontal total = left + right (for column budgeting).
    pub const fn horizontal(&self) -> usize {
        self.left.saturating_add(self.right)
    }

    /// Vertical total = top + bottom (for row budgeting).
    pub const fn vertical(&self) -> usize {
        self.top.saturating_add(self.bottom)
    }
}

/// Visual style of a border edge.
///
/// Terminals render at most one character per cell, so anything that isn't a
/// continuous line collapses to the same single-cell pattern. We keep the
/// distinction in the enum so non-rendering tools (logs, future diagnostics)
/// can still see the original CSS choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    /// Any of `solid` / `dashed` / `dotted` / `double` — render as a single
    /// cell-thick line of box-drawing characters.
    Solid,
}

impl BorderStyle {
    /// Parse the CSS keywords we recognise. Unknown keywords fall back to
    /// [`BorderStyle::Solid`] so the user still sees a visible border when
    /// they write e.g. `border-style: dashed`.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "hidden" => BorderStyle::None,
            _ => BorderStyle::Solid,
        }
    }
}

/// One side of a border: width, style, color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Border {
    pub width: usize,
    pub style: BorderStyle,
    pub color: Option<Color>,
}

impl Border {
    pub const fn new(width: usize, style: BorderStyle, color: Option<Color>) -> Self {
        Self {
            width,
            style,
            color,
        }
    }

    /// Visible thickness of this side (0 when style is `None`).
    pub const fn thickness(&self) -> usize {
        match self.style {
            BorderStyle::Solid => self.width,
            BorderStyle::None => 0,
        }
    }
}

/// All four border edges. Each side has independent width / style / color.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BorderSides {
    pub top: Border,
    pub right: Border,
    pub bottom: Border,
    pub left: Border,
}

impl BorderSides {
    pub const fn all(b: Border) -> Self {
        Self {
            top: b,
            right: b,
            bottom: b,
            left: b,
        }
    }

    /// True if any side has a visible border.
    pub fn any_visible(&self) -> bool {
        self.top.thickness() > 0
            || self.right.thickness() > 0
            || self.bottom.thickness() > 0
            || self.left.thickness() > 0
    }

    /// Sum of the horizontal border widths actually rendered (left + right).
    pub const fn horizontal_thickness(&self) -> usize {
        self.left.thickness().saturating_add(self.right.thickness())
    }

    /// Sum of the vertical border widths actually rendered (top + bottom).
    pub const fn vertical_thickness(&self) -> usize {
        self.top.thickness().saturating_add(self.bottom.thickness())
    }
}

/// The full CSS box-model edge set: padding (inside), border (between), margin
/// (outside).
///
/// Margins are tracked separately from padding because they are not painted —
/// they only affect layout (offset / spacing). Borders and padding together
/// shrink the inner containing block for nested children.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoxEdges {
    pub padding: EdgeSizes,
    pub border: BorderSides,
    pub margin: EdgeSizes,
}

impl BoxEdges {
    /// Horizontal extent of border + padding (columns consumed *inside* the
    /// rect; the rect width itself is unchanged). Children are shrunk by this
    /// amount on each side.
    pub const fn inner_horizontal(&self) -> usize {
        self.padding.horizontal() + self.border.horizontal_thickness()
    }

    /// Vertical extent of border + padding (rows consumed *inside* the rect
    /// when wrapping nested blocks).
    pub const fn inner_vertical(&self) -> usize {
        self.padding.vertical() + self.border.vertical_thickness()
    }
}

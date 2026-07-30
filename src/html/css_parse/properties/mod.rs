//! CSS attribute container and the typed values derived from it.
//!
//! [`CssAttr`] is the raw `IndexMap<String, String>` of declared properties
//! stored on every DOM node; its methods resolve individual values into the
//! typed [`Length`] / `TextAlign` / `FontWeight` / ... enums the layout
//! consumes.
//!
//! ## Module layout
//!
//! ```text
//! properties/
//! ├── mod.rs              ← CssAttr (the main accessor bag) + re-exports
//! └── align_self.rs       ← AlignSelf
//! ```
//!
//! `OuterDisplay` and `FormattingContext` moved to
//! [`crate::layout::layoutbox`] — they describe how a box is rendered, not how
//! CSS declares them, so they live with the layout box definition.

mod align_self;

pub use align_self::AlignSelf;

use crate::html::css_parse::FontContext;
use crate::layout::layoutbox::{FormattingContext, OuterDisplay};

use crossterm::style::Color;
use indexmap::IndexMap;

use crate::html::box_edges::{Border, BorderSides, BoxEdges, EdgeSizes};

use super::values::{
    AlignItems, FlexBasis, FlexDirection, FlexWrap, FontStyleKind, FontWeight, GapSizes,
    JustifyContent, Length, TextAlign, TextDecoration, parse_align_items, parse_border_side,
    parse_border_style, parse_border_width, parse_color_str, parse_edge_sizes, parse_flex_basis,
    parse_flex_direction, parse_flex_wrap, parse_gap_shorthand, parse_justify_content,
    parse_non_negative_number, parse_order, resolve_flex_basis,
};

#[derive(Debug, Clone, Default)]
pub struct CssAttr(pub IndexMap<String, String>);

impl CssAttr {
    pub fn new() -> Self {
        CssAttr(IndexMap::new())
    }
    pub fn add(&mut self, css: CssAttr) {
        self.0.extend(css.0);
    }
    /// Whether `property` has been explicitly set on this node — either by a
    /// matching CSS rule or by inheritance from an ancestor that had it set.
    ///
    /// Used to apply user-agent defaults that must yield to anything the
    /// author has already declared (e.g. `<a>` underlines: apply the default
    /// underline only when neither a CSS rule nor an inherited value has
    /// specified `text-decoration`).
    pub fn is_set(&self, property: &str) -> bool {
        self.0.contains_key(property)
    }
    /// Foreground color (CSS `color`).
    pub fn get_color(&self) -> Option<Color> {
        self.0.get("color").and_then(|c| parse_color_str(c))
    }
    /// Background color (CSS `background-color`, falls back to `background`).
    pub fn get_bg_color(&self) -> Option<Color> {
        self.0
            .get("background-color")
            .or_else(|| self.0.get("background"))
            .and_then(|c| parse_color_str(c))
    }
    /// Text alignment (CSS `text-align`).
    pub fn get_text_align(&self) -> TextAlign {
        match self.0.get("text-align").map(|s| s.as_str()) {
            Some("center") => TextAlign::Center,
            Some("right") => TextAlign::Right,
            _ => TextAlign::Left,
        }
    }
    /// Font weight (CSS `font-weight`).
    pub fn get_font_weight(&self) -> FontWeight {
        match self.0.get("font-weight").map(|s| s.as_str()) {
            Some("bold") | Some("bolder") | Some("700") | Some("800") | Some("900") => {
                FontWeight::Bold
            }
            _ => FontWeight::Normal,
        }
    }
    /// Font style (CSS `font-style`).
    pub fn get_font_style_kind(&self) -> FontStyleKind {
        match self.0.get("font-style").map(|s| s.as_str()) {
            Some("italic") => FontStyleKind::Italic,
            Some("oblique") => FontStyleKind::Oblique,
            _ => FontStyleKind::Normal,
        }
    }
    /// Text decoration (CSS `text-decoration`).
    ///
    /// The CSS property accepts multiple space-separated values such as
    /// `underline line-through`; we parse the subset we render and fold the
    /// combinations into a single variant.
    pub fn get_text_decoration(&self) -> TextDecoration {
        let raw = self
            .0
            .get("text-decoration")
            .map(|s| s.as_str())
            .unwrap_or("");
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        let underline = tokens.iter().any(|t| t.eq_ignore_ascii_case("underline"));
        let line_through = tokens
            .iter()
            .any(|t| t.eq_ignore_ascii_case("line-through"));
        match (underline, line_through) {
            (true, true) => TextDecoration::UnderlineLineThrough,
            (true, false) => TextDecoration::Underline,
            (false, true) => TextDecoration::LineThrough,
            (false, false) => TextDecoration::None,
        }
    }
    /// Element width (CSS `width`).
    ///
    /// - `auto` / missing / unsupported unit → `None` (fill available)
    /// - `40px` / `40` → fixed columns
    /// - `50%` → fraction of terminal width (rounded, minimum 1 column)
    /// - `2em` → cells (resolved against the current element's font-size)
    /// - `2rem` → cells (resolved against the root's font-size)
    /// - `50vw` → fraction of viewport columns (rounded, minimum 1 column)
    ///
    /// `em` / `rem` units are multiplied by `font_size_cells` /
    /// `root_font_size_cells` so they reflect the relative size specified by
    /// CSS. `vw` / `vh` units are multiplied by `viewport_cols` /
    /// `viewport_rows` provided by the layout context. Floats (`1.5em`,
    /// `33.33%`) are rounded to the nearest integer cell — terminals cannot
    /// render half columns. See [`Length::parse`] for the underlying value
    /// type.
    pub fn get_width(
        &self,
        term_columns: usize,
        font_ctx: FontContext,
    ) -> Option<usize> {
        let raw = self.0.get("width")?;
        match Length::parse(raw)? {
            Length::Auto => None,
            len => Some(len.resolve(term_columns, font_ctx)),
        }
    }

    /// Return the raw `font-size` declaration as a [`Length`], if any.
    ///
    /// Callers resolve it via [`Length::resolve`] with the active font-size
    /// context (so `em` values inside `font-size` refer to the inherited
    /// font-size). Returns `None` when the property is unset — use
    /// [`default_font_size_cells_for`](crate::tdom::node::default_font_size_cells_for)
    /// to fall back to the per-element default.
    pub fn get_font_size(&self) -> Option<Length> {
        self.0.get("font-size").and_then(|raw| Length::parse(raw))
    }

    /// Resolve the full set of box-model edges in one call.
    ///
    /// Equivalent to calling [`get_padding`](Self::get_padding),
    /// [`get_margin`](Self::get_margin) and [`get_border`](Self::get_border)
    /// separately. Layout functions use this so they only need to look up
    /// each node's `css_attr` once.
    ///
    /// `font_size_cells` / `root_font_size_cells` are forwarded to every
    /// inner resolution so `em` / `rem` lengths track the element's relative
    /// font-size. `viewport_cols` / `viewport_rows` are forwarded so `vw` /
    /// `vh` lengths resolve against the terminal viewport.
    pub fn get_box_edges(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
    ) -> BoxEdges {
        BoxEdges {
            padding: self.get_padding(containing_width, font_ctx),
            border: self.get_border(containing_width, font_ctx),
            margin: self.get_margin(containing_width, font_ctx),
        }
    }

    /// Resolve all four padding edges from `padding` / `padding-{side}`.
    ///
    /// Per-side longhands (`padding-top`, `padding-right`, `padding-bottom`,
    /// `padding-left`) win over the `padding` shorthand. Shorthand expansion
    /// follows the standard CSS 1-/2-/3-/4-value pattern; tokens we can't
    /// parse as a [`Length`] are treated as `0`.
    ///
    /// `em` / `rem` lengths are scaled by `font_size_cells` /
    /// `root_font_size_cells` so they reflect the relative size of the
    /// element's font. `vw` / `vh` lengths are scaled by `viewport_cols` /
    /// `viewport_rows`.
    pub fn get_padding(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
    ) -> EdgeSizes {
        parse_edge_sizes(&self.0, "padding", containing_width, font_ctx)
    }

    /// Resolve all four margin edges from `margin` / `margin-{side}`.
    ///
    /// `margin: auto` is currently treated as `0` (the terminal has no
    /// centering primitive for margins). Per-side longhands win over the
    /// shorthand.
    pub fn get_margin(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
    ) -> EdgeSizes {
        parse_edge_sizes(&self.0, "margin", containing_width, font_ctx)
    }

    /// Resolve all four border sides from `border` / `border-{side}` /
    /// `border-{prop}`.
    ///
    /// Priority order:
    /// 1. Per-side shorthand (`border-top: 1px solid red`)
    /// 2. Universal properties (`border-width`, `border-style`, `border-color`)
    /// 3. Universal shorthand (`border: 1px solid red`)
    pub fn get_border(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
    ) -> BorderSides {
        // Step 1: collect universal values (lowest priority).
        let universal_width = parse_border_width(
            self.0.get("border-width").map(|s| s.as_str()),
            containing_width,
            font_ctx,
        );
        let universal_style = parse_border_style(self.0.get("border-style").map(|s| s.as_str()));
        let universal_color = self.0.get("border-color").and_then(|s| parse_color_str(s));

        let universal = Border::new(universal_width, universal_style, universal_color);

        // Step 2: parse per-side longhands.
        let top = parse_border_side(
            self.0.get("border-top"),
            self.0.get("border"),
            universal,
            containing_width,
            font_ctx,
        );
        let right = parse_border_side(
            self.0.get("border-right"),
            self.0.get("border"),
            universal,
            containing_width,
            font_ctx,
        );
        let bottom = parse_border_side(
            self.0.get("border-bottom"),
            self.0.get("border"),
            universal,
            containing_width,
            font_ctx,
        );
        let left = parse_border_side(
            self.0.get("border-left"),
            self.0.get("border"),
            universal,
            containing_width,
            font_ctx,
        );

        BorderSides {
            top,
            right,
            bottom,
            left,
        }
    }

    // ------------------------------------------------------------
    // Flexbox accessors
    //
    // Each getter is tolerant: an unparseable value falls back to the
    // CSS initial value so a single typo never silently disables flex
    // for the element. Callers (the flex layout in
    // `layout::flex::tree`) consume the typed result directly.
    // ------------------------------------------------------------

    /// Whether this element opts into flex layout.
    ///
    /// Only the literal `flex` is recognised. `inline-flex` is treated as
    /// `block` (i.e. not flex) for v1 because the renderer is block-only.
    pub fn is_flex(&self) -> bool {
        matches!(
            self.0.get("display").map(|s| s.trim().to_ascii_lowercase()),
            Some(ref v) if v == "flex"
        )
    }

    /// Outer display role from CSS `display`.
    ///
    /// Returns [`OuterDisplay::Inline`] for `inline` / `inline-block` /
    /// `inline-flex` / `inline-grid`, and [`OuterDisplay::Block`] for the
    /// block-level equivalents (`block` / `flex` / `grid`) plus the default
    /// when `display` is missing.
    pub fn outer_display(&self) -> OuterDisplay {
        match self
            .0
            .get("display")
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("inline") | Some("inline-block") | Some("inline-flex") | Some("inline-grid") => {
                OuterDisplay::Inline
            }
            _ => OuterDisplay::Block,
        }
    }

    /// Inner formatting context from CSS `display`.
    ///
    /// Returns [`FormattingContext::Flex`] for `flex` / `inline-flex`,
    /// [`FormattingContext::Grid`] for `grid` / `inline-grid`, and
    /// [`FormattingContext::Flow`] for everything else (including the default
    /// when `display` is missing).
    pub fn inner_formatting_context(&self) -> FormattingContext {
        match self
            .0
            .get("display")
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("flex") | Some("inline-flex") => FormattingContext::Flex,
            Some("grid") | Some("inline-grid") => FormattingContext::Grid,
            _ => FormattingContext::Flow,
        }
    }

    /// `flex-direction` (defaults to `Row`).
    pub fn get_flex_direction(&self) -> FlexDirection {
        self.0
            .get("flex-direction")
            .and_then(|v| parse_flex_direction(v))
            .unwrap_or(FlexDirection::Row)
    }

    /// `flex-wrap` (defaults to `NoWrap`).
    pub fn get_flex_wrap(&self) -> FlexWrap {
        self.0
            .get("flex-wrap")
            .and_then(|v| parse_flex_wrap(v))
            .unwrap_or(FlexWrap::NoWrap)
    }

    /// `justify-content` (defaults to `FlexStart`).
    pub fn get_justify_content(&self) -> JustifyContent {
        self.0
            .get("justify-content")
            .and_then(|v| parse_justify_content(v))
            .unwrap_or(JustifyContent::FlexStart)
    }

    /// `align-items` for the container (defaults to `Stretch`).
    pub fn get_align_items(&self) -> AlignItems {
        self.0
            .get("align-items")
            .and_then(|v| parse_align_items(v))
            .unwrap_or(AlignItems::Stretch)
    }

    /// `align-self` for a single flex item (defaults to `Auto`).
    ///
    /// `Auto` means "inherit the parent's `align-items`"; the layout
    /// applies the fallback at item-resolution time so a parent change
    /// is honoured without re-parsing the child.
    pub fn get_align_self(&self) -> AlignSelf {
        match self
            .0
            .get("align-self")
            .map(|s| s.trim().to_ascii_lowercase())
        {
            Some(v) if v == "auto" => AlignSelf::Auto,
            Some(_) => AlignSelf::Value(
                self.0
                    .get("align-self")
                    .and_then(|v| parse_align_items(v))
                    .unwrap_or(AlignItems::Stretch),
            ),
            None => AlignSelf::Auto,
        }
    }

    /// `align-content` for multi-line flex containers (defaults to
    /// `FlexStart`).
    pub fn get_align_content(&self) -> JustifyContent {
        self.0
            .get("align-content")
            .and_then(|v| parse_justify_content(v))
            .unwrap_or(JustifyContent::FlexStart)
    }

    /// `flex-grow` (defaults to `0`).
    pub fn get_flex_grow(&self) -> f32 {
        self.0
            .get("flex-grow")
            .and_then(|v| parse_non_negative_number(v))
            .unwrap_or(0.0)
    }

    /// `flex-shrink` (defaults to `1`).
    pub fn get_flex_shrink(&self) -> f32 {
        self.0
            .get("flex-shrink")
            .and_then(|v| parse_non_negative_number(v))
            .unwrap_or(1.0)
    }

    /// `flex-basis` (defaults to `Auto`).
    pub fn get_flex_basis(&self) -> FlexBasis {
        self.0
            .get("flex-basis")
            .and_then(|v| parse_flex_basis(v))
            .unwrap_or(FlexBasis::Auto)
    }

    /// Resolve `flex-basis` against the containing main-axis size.
    ///
    /// Returns `None` for `Auto` so the caller can apply its own fallback
    /// chain (`width` / `height` → content measure → equal share).
    pub fn get_flex_basis_resolved(
        &self,
        containing_main: usize,
        font_ctx: FontContext,
    ) -> Option<usize> {
        resolve_flex_basis(self.get_flex_basis(), containing_main, font_ctx)
    }

    /// `order` (defaults to `0`).
    pub fn get_order(&self) -> i32 {
        self.0
            .get("order")
            .and_then(|v| parse_order(v))
            .unwrap_or(0)
    }

    /// Resolve `gap` / `row-gap` / `column-gap` into a [`GapSizes`].
    ///
    /// `gap` is the shorthand (1- or 2-value). `row-gap` / `column-gap`
    /// longhands override the corresponding shorthand component when
    /// both are present. Anything unparseable collapses to `0`.
    ///
    /// `vw` / `vh` lengths resolve against the viewport passed in; percentage
    /// gap is still resolved against `usize::MAX` (intentional — see
    /// `doc/layout.md`, "百分比 gap" — the flex container's actual axis
    /// size is not in scope here).
    pub fn get_gap(&self, font_ctx: FontContext) -> GapSizes {
        let shorthand = self
            .0
            .get("gap")
            .and_then(|v| parse_gap_shorthand(v, font_ctx));
        let row = self
            .0
            .get("row-gap")
            .and_then(|v| Length::parse(v))
            .map(|l| l.resolve(usize::MAX, font_ctx))
            .or_else(|| shorthand.map(|g| g.row))
            .unwrap_or(0);
        let column = self
            .0
            .get("column-gap")
            .and_then(|v| Length::parse(v))
            .map(|l| l.resolve(usize::MAX, font_ctx))
            .or_else(|| shorthand.map(|g| g.column))
            .unwrap_or(0);
        GapSizes { row, column }
    }

    /// Element height (CSS `height`).
    ///
    /// Mirrors [`get_width`](Self::get_width) for the cross-axis size. Used
    /// by the flex layout to detect an explicit container height so
    /// `align-items: stretch` can take effect (stretch is otherwise a no-op
    /// in v1 — see `flex::tree` for the rationale).
    pub fn get_height(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
    ) -> Option<usize> {
        let raw = self.0.get("height")?;
        match Length::parse(raw)? {
            Length::Auto => None,
            len => Some(len.resolve(containing_width, font_ctx)),
        }
    }
}
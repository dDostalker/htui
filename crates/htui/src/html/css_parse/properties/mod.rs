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

use crate::html::css_parse::{
    FontContext, InsetSides, LengthContext, Positioning, Viewport, parse_position,
};
use crate::layout::layoutbox::{FormattingContext, OuterDisplay};
use crate::tdom::node::NodeType;

use crossterm::style::Color;
use indexmap::IndexMap;

use crate::html::box_edges::{Border, BorderSides, BoxEdges, EdgeSizes};

use super::values::{
    AlignItems, BoxSizing, CornerRadii, CssDisplay, FlexBasis, FlexDirection, FlexWrap,
    FontStyleKind, FontWeight, GapSizes, JustifyContent, Length, LineHeight, OverflowWrap,
    TextAlign, TextDecoration, TextTransform, Visibility, WhiteSpace, WordBreak, parse_align_items,
    parse_border_radius, parse_border_side, parse_border_style, parse_border_width,
    parse_box_sizing, parse_color_str, parse_edge_sizes, parse_flex_basis, parse_flex_direction,
    parse_flex_wrap, parse_gap_shorthand, parse_justify_content, parse_line_height,
    parse_non_negative_number, parse_order, parse_overflow_wrap, parse_text_transform,
    parse_visibility, parse_white_space, parse_word_break, resolve_flex_basis,
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
    /// Resolve a property with `parse`, falling back to `default` when the
    /// property is absent or unparseable.
    fn parse_or<T>(&self, key: &str, parse: fn(&str) -> Option<T>, default: T) -> T {
        self.0
            .get(key)
            .and_then(|s| parse(s.as_str()))
            .unwrap_or(default)
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
    /// Whitespace handling (CSS `white-space`).
    ///
    /// Resolution order mirrors the cascade:
    /// 1. An explicit `white-space` declaration on the node.
    /// 2. The element-type UA default (`<pre>` ships with
    ///    [`WhiteSpace::Pre`] so verbatim whitespace survives the
    ///    parser pass).
    /// 3. The browser default of [`WhiteSpace::Normal`] (collapse +
    ///    wrap).
    pub fn get_white_space(&self, node_type: &NodeType) -> WhiteSpace {
        self.0
            .get("white-space")
            .and_then(|s| parse_white_space(s.as_str()))
            .or_else(|| node_type.default_white_space())
            .unwrap_or_default()
    }
    /// Word-break behaviour for CJK and long-word overflow
    /// (CSS `word-break`).
    ///
    /// Defaults to [`WordBreak::Normal`] (allow CJK character breaks,
    /// disallow mid-word breaks for unbreakable overflows). The
    /// per-element break decisions are made by the layout-phase line
    /// breaker.
    pub fn get_word_break(&self) -> WordBreak {
        self.parse_or("word-break", parse_word_break, WordBreak::default())
    }
    /// Overflow-wrap behaviour (CSS `overflow-wrap`, legacy
    /// `word-wrap`).
    ///
    /// Accepts the legacy `word-wrap` keyword in addition to the
    /// standard `overflow-wrap` so older stylesheets still apply.
    /// Defaults to [`OverflowWrap::Normal`] (no mid-word break for
    /// unbreakable overflows).
    pub fn get_overflow_wrap(&self) -> OverflowWrap {
        self.0
            .get("overflow-wrap")
            .or_else(|| self.0.get("word-wrap"))
            .and_then(|s| parse_overflow_wrap(s.as_str()))
            .unwrap_or_default()
    }
    /// Element visibility (CSS `visibility`). Inherited, so a child without
    /// an explicit value picks up its parent's. `Hidden` keeps the box's
    /// layout space but paints nothing; a descendant with an explicit
    /// `visible` still paints (each node decides at paint time).
    pub fn get_visibility(&self) -> Visibility {
        self.0
            .get("visibility")
            .and_then(|s| parse_visibility(s.as_str()))
            .unwrap_or_default()
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
        viewport: Viewport,
    ) -> Option<usize> {
        let raw = self.0.get("width")?;
        match Length::parse(raw)? {
            Length::Auto => None,
            len => Some(len.resolve(LengthContext {
                containing: term_columns,
                font: font_ctx,
                viewport,
            })),
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
        viewport: Viewport,
    ) -> BoxEdges {
        BoxEdges {
            padding: self.get_padding(containing_width, font_ctx, viewport),
            border: self.get_border(containing_width, font_ctx, viewport),
            margin: self.get_margin(containing_width, font_ctx, viewport),
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
        viewport: Viewport,
    ) -> EdgeSizes {
        parse_edge_sizes(&self.0, "padding", containing_width, font_ctx, viewport)
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
        viewport: Viewport,
    ) -> EdgeSizes {
        parse_edge_sizes(&self.0, "margin", containing_width, font_ctx, viewport)
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
        viewport: Viewport,
    ) -> BorderSides {
        // Step 1: collect universal values (lowest priority).
        let universal_width = parse_border_width(
            self.0.get("border-width").map(|s| s.as_str()),
            containing_width,
            font_ctx,
            viewport,
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
            viewport,
        );
        let right = parse_border_side(
            self.0.get("border-right"),
            self.0.get("border"),
            universal,
            containing_width,
            font_ctx,
            viewport,
        );
        let bottom = parse_border_side(
            self.0.get("border-bottom"),
            self.0.get("border"),
            universal,
            containing_width,
            font_ctx,
            viewport,
        );
        let left = parse_border_side(
            self.0.get("border-left"),
            self.0.get("border"),
            universal,
            containing_width,
            font_ctx,
            viewport,
        );

        BorderSides {
            top,
            right,
            bottom,
            left,
        }
    }

    // ------------------------------------------------------------
    // `display` accessor
    //
    // `display` is the single source of truth for the box's outer
    // role and inner formatting context. The author-declared value is
    // optional; the UA default for the element type is used when the
    // property is absent. Callers (the generation phase, the inline
    // runs walker, the flex item intrinsic-size helper, …) consume
    // the typed `CssDisplay` directly. Per-axis views are exposed via
    // [`outer_display`] and [`inner_formatting_context`].
    // ------------------------------------------------------------

    /// Raw author-declared `display` value as a typed enum, or `None`
    /// when the property is absent.
    pub fn display(&self) -> Option<CssDisplay> {
        self.0
            .get("display")
            .map(String::as_str)
            .and_then(CssDisplay::parse)
    }

    /// Author-declared `display` if present, otherwise the UA default
    /// for `node_type`. `display: none` and `display: contents` are
    /// preserved here so the caller can branch on them.
    fn display_or_ua(&self, node_type: &NodeType) -> CssDisplay {
        self.display()
            .unwrap_or_else(|| node_type.ua_default_display())
    }

    /// Outer display role derived from CSS `display` (with UA default
    /// fallback). Returns a `BlockLevel { container: bool } | Inline
    /// { block: bool }` so geometry can tell apart a block-level box
    /// from a block container (a box that establishes a new FC for its
    /// children).
    pub fn outer_display(&self, node_type: &NodeType) -> OuterDisplay {
        OuterDisplay::from_css_display(self.display_or_ua(node_type))
    }

    /// Inner formatting context derived from CSS `display` (with UA
    /// default fallback).
    pub fn inner_formatting_context(&self, node_type: &NodeType) -> FormattingContext {
        let d = self.display_or_ua(node_type);
        if d.is_table_family() {
            FormattingContext::Table
        } else if d.is_flex() {
            FormattingContext::Flex
        } else if matches!(d, CssDisplay::Grid | CssDisplay::InlineGrid) {
            FormattingContext::Grid
        } else {
            FormattingContext::Flow
        }
    }

    /// `flex-direction` (defaults to `Row`).
    pub fn get_flex_direction(&self) -> FlexDirection {
        self.parse_or("flex-direction", parse_flex_direction, FlexDirection::Row)
    }

    /// `flex-wrap` (defaults to `NoWrap`).
    pub fn get_flex_wrap(&self) -> FlexWrap {
        self.parse_or("flex-wrap", parse_flex_wrap, FlexWrap::NoWrap)
    }

    /// `justify-content` (defaults to `FlexStart`).
    pub fn get_justify_content(&self) -> JustifyContent {
        self.parse_or(
            "justify-content",
            parse_justify_content,
            JustifyContent::FlexStart,
        )
    }

    /// `align-items` for the container (defaults to `Stretch`).
    pub fn get_align_items(&self) -> AlignItems {
        self.parse_or("align-items", parse_align_items, AlignItems::Stretch)
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
        self.parse_or(
            "align-content",
            parse_justify_content,
            JustifyContent::FlexStart,
        )
    }

    /// `flex-grow` (defaults to `0`).
    pub fn get_flex_grow(&self) -> f32 {
        self.parse_or("flex-grow", parse_non_negative_number, 0.0)
    }

    /// `flex-shrink` (defaults to `1`).
    pub fn get_flex_shrink(&self) -> f32 {
        self.parse_or("flex-shrink", parse_non_negative_number, 1.0)
    }

    /// `flex-basis` (defaults to `Auto`).
    pub fn get_flex_basis(&self) -> FlexBasis {
        self.parse_or("flex-basis", parse_flex_basis, FlexBasis::Auto)
    }

    /// Resolve `flex-basis` against the containing main-axis size.
    ///
    /// Returns `None` for `Auto` so the caller can apply its own fallback
    /// chain (`width` / `height` → content measure → equal share).
    pub fn get_flex_basis_resolved(
        &self,
        containing_main: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        resolve_flex_basis(self.get_flex_basis(), containing_main, font_ctx, viewport)
    }

    /// `order` (defaults to `0`).
    pub fn get_order(&self) -> i32 {
        self.parse_or("order", parse_order, 0)
    }

    /// Resolve `gap` / `row-gap` / `column-gap` into a [`GapSizes`].
    ///
    /// `gap` is the shorthand (1- or 2-value). `row-gap` / `column-gap`
    /// longhands override the corresponding shorthand component when
    /// both are present. Anything unparseable collapses to `0`.
    ///
    /// `vw` / `vh` lengths resolve against the viewport passed in; percentage
    /// gap is still resolved against `usize::MAX` (intentional — see
    /// `doc/layout.md`, "percentage gap" — the flex container's actual axis
    /// size is not in scope here).
    pub fn get_gap(&self, font_ctx: FontContext, viewport: Viewport) -> GapSizes {
        let shorthand = self
            .0
            .get("gap")
            .and_then(|v| parse_gap_shorthand(v, font_ctx, viewport));
        let row = self
            .0
            .get("row-gap")
            .and_then(|v| Length::parse(v))
            .map(|l| {
                l.resolve(LengthContext {
                    containing: usize::MAX,
                    font: font_ctx,
                    viewport,
                })
            })
            .or_else(|| shorthand.map(|g| g.row))
            .unwrap_or(0);
        let column = self
            .0
            .get("column-gap")
            .and_then(|v| Length::parse(v))
            .map(|l| {
                l.resolve(LengthContext {
                    containing: usize::MAX,
                    font: font_ctx,
                    viewport,
                })
            })
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
        viewport: Viewport,
    ) -> Option<usize> {
        let raw = self.0.get("height")?;
        match Length::parse(raw)? {
            Length::Auto => None,
            len => Some(len.resolve(LengthContext {
                containing: containing_width,
                font: font_ctx,
                viewport,
            })),
        }
    }

    /// Text case transform (CSS `text-transform`). `none` when unset.
    ///
    /// Inherited, so the cascade already merged an ancestor's value into this
    /// node's `css_attr` when applicable.
    pub fn get_text_transform(&self) -> TextTransform {
        self.parse_or(
            "text-transform",
            parse_text_transform,
            TextTransform::default(),
        )
    }

    /// Line-box height (CSS `line-height`). `normal` when unset.
    ///
    /// Inherited, so the cascade already merged an ancestor's value into this
    /// node's `css_attr` when applicable. Resolve the row count with
    /// [`resolve_line_height_cells`](crate::html::css_parse::resolve_line_height_cells).
    pub fn get_line_height(&self) -> LineHeight {
        self.parse_or("line-height", parse_line_height, LineHeight::Normal)
    }

    /// Box sizing model (CSS `box-sizing`).
    ///
    /// The engine's default is `border-box` (see [`BoxSizing`]) so unset
    /// preserves the historical width semantics; an explicit `content-box`
    /// switches `width` / `height` to CSS content-box semantics.
    pub fn get_box_sizing(&self) -> BoxSizing {
        self.parse_or("box-sizing", parse_box_sizing, BoxSizing::default())
    }

    /// Corner radii (CSS `border-radius` shorthand).
    ///
    /// Unset yields all-zero radii (square corners). Lengths resolve like any
    /// other edge value (`%` against the containing width).
    pub fn get_border_radius(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> CornerRadii {
        parse_border_radius(&self.0, containing_width, font_ctx, viewport)
    }

    /// Minimum width (CSS `min-width`). `None` when unset / `auto`.
    pub fn get_min_width(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        self.resolve_min_max_len("min-width", containing_width, font_ctx, viewport)
    }

    /// Maximum width (CSS `max-width`). `None` when unset / `none` / `auto`.
    pub fn get_max_width(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        self.resolve_min_max_len("max-width", containing_width, font_ctx, viewport)
    }

    /// Minimum height (CSS `min-height`). `None` when unset / `auto`.
    pub fn get_min_height(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        self.resolve_min_max_len("min-height", containing_width, font_ctx, viewport)
    }

    /// Maximum height (CSS `max-height`). `None` when unset / `none` / `auto`.
    pub fn get_max_height(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        self.resolve_min_max_len("max-height", containing_width, font_ctx, viewport)
    }

    /// Shared resolution for the four min/max size longhands.
    ///
    /// `auto` (or `none`, which fails [`Length::parse`]) means "no
    /// constraint" — the property's initial value — so both map to `None`
    /// and the caller leaves the size unconstrained.
    fn resolve_min_max_len(
        &self,
        key: &str,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        let raw = self.0.get(key)?;
        match Length::parse(raw)? {
            Length::Auto => None,
            len => Some(len.resolve(LengthContext {
                containing: containing_width,
                font: font_ctx,
                viewport,
            })),
        }
    }

    /// Resolved `position` + insets + `z-index` (CSS Positioned Layout).
    ///
    /// Insets resolve against `containing_width` so percentage / `vw`
    /// lengths have a containing block; `auto` sides become `None`. `auto`
    /// `z-index` (or an unparseable value) becomes `None`, which paints the
    /// box at stacking order 0.
    pub fn get_positioning(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Positioning {
        let position = self
            .0
            .get("position")
            .and_then(|v| parse_position(v))
            .unwrap_or_default();
        let resolve_inset = |key: &str| {
            self.0.get(key).and_then(|raw| match Length::parse(raw)? {
                Length::Auto => None,
                len => Some(len.resolve(LengthContext {
                    containing: containing_width,
                    font: font_ctx,
                    viewport,
                })),
            })
        };
        let z_index = self
            .0
            .get("z-index")
            .and_then(|v| v.trim().parse::<i32>().ok());
        Positioning {
            position,
            inset: InsetSides {
                top: resolve_inset("top"),
                right: resolve_inset("right"),
                bottom: resolve_inset("bottom"),
                left: resolve_inset("left"),
            },
            z_index,
        }
    }
}

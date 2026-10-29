//! CSS attribute container and the typed values derived from it.
//!
//! [`CssAttr`] is the raw `IndexMap<Arc<str>, Arc<str>>` of declared
//! properties stored on every DOM node; its methods resolve individual values
//! into the typed [`Length`] / `TextAlign` / `FontWeight` / ... enums the
//! layout consumes.
//!
//! Declaration keys and values are shared (`Arc<str>`) so the cascade can copy
//! a rule's declarations into a node by bumping a reference count instead of
//! deep-cloning every `String` on every frame.
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

use std::sync::Arc;

use crossterm::style::Color;
use indexmap::IndexMap;

use crate::html::box_edges::{
    Border, BorderSides, BorderStyle, BoxEdges, EdgeAuto, EdgeSizes, MarginSizes,
};

use super::values::{
    AlignContent, AlignItems, BoxSizing, CaptionSide, ColumnFill, ColumnsResolved, CornerRadii,
    CssDisplay, EmptyCells, FlexBasis, FlexDirection, FlexShorthand, FlexWrap, FontStyleKind,
    FontWeight, GapSizes, JustifyContent, Length, LineHeight, ListStyleType, OverflowWrap,
    TableLayout, TextAlign, TextDecoration, TextTransform, UserSelect, Visibility, WhiteSpace,
    WordBreak, merge_border_longhand, parse_align_content, parse_align_items, parse_background_str,
    parse_border_radius, parse_border_side, parse_border_style, parse_border_width,
    parse_box_sizing, parse_caption_side, parse_color_str, parse_column_count, parse_column_fill,
    parse_column_rule, parse_columns_shorthand, parse_edge_auto, parse_edge_sizes,
    parse_empty_cells, parse_flex_basis, parse_flex_direction, parse_flex_flow,
    parse_flex_shorthand, parse_flex_wrap, parse_gap_shorthand, parse_justify_content,
    parse_line_height, parse_list_style_type, parse_margin_sizes, parse_non_negative_number,
    parse_order, parse_overflow_wrap, parse_table_layout, parse_text_transform, parse_user_select,
    parse_visibility, parse_white_space, parse_word_break, resolve_flex_basis,
};

#[derive(Debug, Clone, Default)]
pub struct CssAttr(pub IndexMap<Arc<str>, Arc<str>>);

/// Convert an owned `String`-keyed declaration map into a shared [`CssAttr`].
/// Each entry is moved into an `Arc<str>` so the value is referenced, not
/// copied, on every later cascade pass.
impl From<IndexMap<String, String>> for CssAttr {
    fn from(map: IndexMap<String, String>) -> Self {
        CssAttr(
            map.into_iter()
                .map(|(k, v)| (Arc::from(k), Arc::from(v)))
                .collect(),
        )
    }
}

impl CssAttr {
    /// Create an empty declaration set.
    pub fn new() -> Self {
        CssAttr(IndexMap::new())
    }
    /// Merge `css`'s declarations into this attribute set, keeping whatever this
    /// set already had when a property is absent from `css`. Because keys and
    /// values are `Arc<str>`, each entry is copied by bumping a reference count —
    /// no intermediate map allocation, unlike cloning the whole `CssAttr` first.
    pub fn add(&mut self, css: &CssAttr) {
        for (k, v) in &css.0 {
            self.0.insert(Arc::clone(k), Arc::clone(v));
        }
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
            .and_then(|s| parse(s.as_ref()))
            .unwrap_or(default)
    }
    /// Foreground color (CSS `color`).
    pub fn get_color(&self) -> Option<Color> {
        self.0.get("color").and_then(|c| parse_color_str(c))
    }
    /// Whether the element's background is clipped to its text glyphs.
    pub fn background_clips_to_text(&self) -> bool {
        self.0.get("background-clip").is_some_and(|value| {
            value
                .split_whitespace()
                .last()
                .is_some_and(|keyword| keyword.eq_ignore_ascii_case("text"))
        })
    }

    /// Cursor colour (CSS `caret-color`). `None` for `auto` / unset — the
    /// painter then uses the default inverted-cell caret.
    pub fn get_caret_color(&self) -> Option<Color> {
        self.0.get("caret-color").and_then(|v| {
            if v.trim().eq_ignore_ascii_case("auto") {
                None
            } else {
                parse_color_str(v)
            }
        })
    }
    /// Background color (CSS `background-color`, falls back to `background`).
    pub fn get_bg_color(&self) -> Option<Color> {
        self.0
            .get("background-color")
            .or_else(|| self.0.get("background"))
            .and_then(|c| parse_color_str(c))
    }
    /// Background paint source, including CSS gradients.
    pub fn get_background(&self) -> Option<crate::html::css_parse::Background> {
        self.0
            .get("background-color")
            .or_else(|| self.0.get("background"))
            .and_then(|value| parse_background_str(value))
    }
    /// Text alignment (CSS `text-align`).
    ///
    /// Handles the four logical keywords in addition to the physical ones.
    /// `start` maps to `Left` and `end` maps to `Right` (the engine is LTR
    /// only); `justify` collapses to `Left` (no inter-word justification in a
    /// monospace grid). CSS keywords are ASCII case-insensitive, so the
    /// comparison is case-insensitive.
    pub fn get_text_align(&self) -> TextAlign {
        match self.0.get("text-align").map(|s| s.as_ref()) {
            Some(s) if s.trim().eq_ignore_ascii_case("center") => TextAlign::Center,
            Some(s)
                if s.trim().eq_ignore_ascii_case("right")
                    || s.trim().eq_ignore_ascii_case("end") =>
            {
                TextAlign::Right
            }
            // `left`, `start`, `justify` (collapsed to left) and everything
            // else fall through to the default.
            _ => TextAlign::Left,
        }
    }
    /// Font weight (CSS `font-weight`).
    ///
    /// Terminal output has two weights, so numeric weights map to the nearest
    /// bucket: `600..=900` (and the `bold` / `bolder` keywords) are bold, the
    /// rest are regular. CSS keywords and the `to_ascii_lowercase` comparison
    /// are ASCII case-insensitive.
    pub fn get_font_weight(&self) -> FontWeight {
        match self.0.get("font-weight").map(|s| s.as_ref()) {
            Some(s) => {
                let w = s.trim().to_ascii_lowercase();
                match w.as_str() {
                    "bold" | "bolder" | "600" | "700" | "800" | "900" => FontWeight::Bold,
                    _ => FontWeight::Normal,
                }
            }
            None => FontWeight::Normal,
        }
    }
    /// Font style (CSS `font-style`).
    ///
    /// CSS keywords are ASCII case-insensitive; `italic` / `oblique` both
    /// render as italic in the terminal.
    pub fn get_font_style_kind(&self) -> FontStyleKind {
        match self.0.get("font-style").map(|s| s.as_ref()) {
            Some(s) => {
                let v = s.trim().to_ascii_lowercase();
                match v.as_str() {
                    "italic" => FontStyleKind::Italic,
                    "oblique" => FontStyleKind::Oblique,
                    _ => FontStyleKind::Normal,
                }
            }
            None => FontStyleKind::Normal,
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
            .map(|s| s.as_ref())
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
            .and_then(|s| parse_white_space(s.as_ref()))
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
            .and_then(|s| parse_overflow_wrap(s.as_ref()))
            .unwrap_or_default()
    }
    /// Text selection behaviour (CSS `user-select`).
    ///
    /// The property itself is not inherited. Callers resolve `Auto` against
    /// ancestors so an explicit `text` descendant can opt back in under a
    /// `none` container, matching CSS UI used-value resolution.
    pub fn get_user_select(&self) -> UserSelect {
        self.0
            .get("user-select")
            .and_then(|s| parse_user_select(s.as_ref()))
            .unwrap_or_default()
    }

    /// Element visibility (CSS `visibility`). Inherited, so a child without
    /// an explicit value picks up its parent's. `Hidden` keeps the box's
    /// layout space but paints nothing; a descendant with an explicit
    /// `visible` still paints (each node decides at paint time).
    pub fn get_visibility(&self) -> Visibility {
        self.0
            .get("visibility")
            .and_then(|s| parse_visibility(s.as_ref()))
            .unwrap_or_default()
    }
    /// Resolved CSS `overflow` for the box's content. `visible` (the CSS
    /// default) lets content paint beyond the content box; `hidden`,
    /// `auto`, `scroll` and `clip` clip it there — the container-scroll
    /// keywords have no terminal counterpart (the engine scrolls the whole
    /// document), so they degenerate to clipping. When the two axes
    /// disagree, the clipping axis wins (`overflow-x: hidden` alone clips).
    pub fn get_overflow(&self) -> crate::html::css_parse::Overflow {
        use crate::html::css_parse::{Overflow, parse_overflow};
        let axis = |key: &str| {
            self.0
                .get(key)
                .and_then(|s| parse_overflow(s.as_ref()))
                .unwrap_or(Overflow::Visible)
        };
        if axis("overflow") == Overflow::Clip
            || axis("overflow-x") == Overflow::Clip
            || axis("overflow-y") == Overflow::Clip
        {
            Overflow::Clip
        } else {
            Overflow::Visible
        }
    }

    /// Whether the vertical axis explicitly prevents viewport scrolling.
    /// This is the root/body propagation rule for `overflow: hidden` / `clip`;
    /// `auto` / `scroll` still create a scrollable viewport.
    pub fn locks_vertical_scroll(&self) -> bool {
        let axis = |key: &str| {
            self.0
                .get(key)
                .map(|s| {
                    let value = s.trim().to_ascii_lowercase();
                    value == "hidden" || value == "clip"
                })
                .unwrap_or(false)
        };
        axis("overflow-y") || (!self.0.contains_key("overflow-y") && axis("overflow"))
    }

    /// Whether the resolved vertical overflow is `visible` when no axis value
    /// prevents scrolling. Root propagation consults this before applying the
    /// body rule.
    pub fn vertical_scroll_visible(&self) -> bool {
        let key = if self.0.contains_key("overflow-y") {
            "overflow-y"
        } else {
            "overflow"
        };
        self.0.get(key).is_none_or(|s| {
            let value = s.trim().to_ascii_lowercase();
            value != "hidden" && value != "clip"
        })
    }

    /// Whether this box establishes a vertical browser-style scroll
    /// container. `auto` shows a scrollbar only when content overflows;
    /// `scroll` reserves one whenever the box has a fixed height.
    pub fn scrolls_vertically(&self) -> bool {
        let axis = |key: &str| {
            self.0
                .get(key)
                .map(|s| {
                    s.trim().eq_ignore_ascii_case("auto") || s.trim().eq_ignore_ascii_case("scroll")
                })
                .unwrap_or(false)
        };
        axis("overflow-y") || axis("overflow")
    }
    /// Element width (CSS `width`).
    ///
    /// - `auto` / missing / unsupported unit → `None` (fill available)
    /// - `40px` / bare `40` → px, divided by [`PX_PER_CELL`](crate::html::css_parse::PX_PER_CELL) (e.g. 40px → 5 cells)
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
    /// [`effective_font_size_cells`](crate::tdom::node::effective_font_size_cells)
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
    /// `margin: auto` resolves to `0` here; the per-side `auto` flags (see
    /// [`Self::get_margin_auto`]) are handed to the layout phase, which distributes
    /// the containing block's free space — both horizontal auto margins
    /// center the block, a single one absorbs the remainder. Per-side
    /// longhands win over the shorthand.
    ///
    /// The horizontal edges (`left` / `right`) are **signed**: negative
    /// margins pull the box toward (or past) its neighbours, and a negative
    /// horizontal pair widens the available space. The vertical edges
    /// (`top` / `bottom`) are clamped to `0` when negative — the cursor
    /// model has no overlapping-row painting.
    pub fn get_margin(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> MarginSizes {
        parse_margin_sizes(&self.0, containing_width, font_ctx, viewport)
    }

    /// Per-side `auto` flags for the margin edges (CSS `margin: 0 auto`).
    ///
    /// Block-flow layout uses only the horizontal flags: both auto margins
    /// center the box within its containing block, a single auto margin
    /// absorbs all the free space on its side. Vertical auto margins stay
    /// `0` in block flow (CSS).
    pub fn get_margin_auto(&self) -> EdgeAuto {
        parse_edge_auto(&self.0, "margin")
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
            self.0.get("border-width").map(|s| s.as_ref()),
            containing_width,
            font_ctx,
            viewport,
        );
        let universal_style = parse_border_style(self.0.get("border-style").map(|s| s.as_ref()));
        let universal_color = self.0.get("border-color").and_then(|s| parse_color_str(s));

        let universal = Border::new(universal_width, universal_style, universal_color);

        // Step 2: parse per-side longhands.
        let top = merge_border_longhand(
            parse_border_side(
                self.0.get("border-top"),
                self.0.get("border"),
                universal,
                containing_width,
                font_ctx,
                viewport,
            ),
            &self.0,
            "top",
            containing_width,
            font_ctx,
            viewport,
        );
        let right = merge_border_longhand(
            parse_border_side(
                self.0.get("border-right"),
                self.0.get("border"),
                universal,
                containing_width,
                font_ctx,
                viewport,
            ),
            &self.0,
            "right",
            containing_width,
            font_ctx,
            viewport,
        );
        let bottom = merge_border_longhand(
            parse_border_side(
                self.0.get("border-bottom"),
                self.0.get("border"),
                universal,
                containing_width,
                font_ctx,
                viewport,
            ),
            &self.0,
            "bottom",
            containing_width,
            font_ctx,
            viewport,
        );
        let left = merge_border_longhand(
            parse_border_side(
                self.0.get("border-left"),
                self.0.get("border"),
                universal,
                containing_width,
                font_ctx,
                viewport,
            ),
            &self.0,
            "left",
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
            .map(|s| s.as_ref())
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

    /// `flex-direction` (defaults to `Row`). Falls back to the `flex-flow`
    /// shorthand's direction component when `flex-direction` is unset.
    pub fn get_flex_direction(&self) -> FlexDirection {
        match self.0.get("flex-direction") {
            Some(v) => parse_flex_direction(v).unwrap_or(FlexDirection::Row),
            None => self
                .0
                .get("flex-flow")
                .and_then(|v| parse_flex_flow(v))
                .map(|(dir, _)| dir)
                .unwrap_or(FlexDirection::Row),
        }
    }

    /// `flex-wrap` (defaults to `NoWrap`). Falls back to the `flex-flow`
    /// shorthand's wrap component when `flex-wrap` is unset.
    pub fn get_flex_wrap(&self) -> FlexWrap {
        match self.0.get("flex-wrap") {
            Some(v) => parse_flex_wrap(v).unwrap_or(FlexWrap::NoWrap),
            None => self
                .0
                .get("flex-flow")
                .and_then(|v| parse_flex_flow(v))
                .and_then(|(_, wrap)| wrap)
                .unwrap_or(FlexWrap::NoWrap),
        }
    }

    /// `justify-content` (defaults to `FlexStart`). The `place-content`
    /// shorthand is expanded into this longhand at parse time, so the normal
    /// later-declaration-wins cascade ordering applies.
    pub fn get_justify_content(&self) -> JustifyContent {
        self.0
            .get("justify-content")
            .and_then(|v| parse_justify_content(v))
            .unwrap_or(JustifyContent::FlexStart)
    }

    /// `align-items` for the container (defaults to `Stretch`). The
    /// `place-items` shorthand is expanded into this longhand at parse time.
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
    /// is honoured without re-parsing the child. The `place-self`
    /// shorthand is expanded into this longhand at parse time.
    pub fn get_align_self(&self) -> AlignSelf {
        match self
            .0
            .get("align-self")
            .map(|s| s.trim().to_ascii_lowercase())
        {
            Some(v) if v == "auto" => AlignSelf::Auto,
            Some(v) => AlignSelf::Value(parse_align_items(&v).unwrap_or(AlignItems::Stretch)),
            None => AlignSelf::Auto,
        }
    }

    /// `align-content` for multi-line flex containers. Defaults to
    /// `Stretch` (the CSS initial value: lines fill the free cross space).
    /// The `place-content` shorthand is expanded into this longhand at
    /// parse time.
    pub fn get_align_content(&self) -> AlignContent {
        self.0
            .get("align-content")
            .and_then(|v| parse_align_content(v))
            .unwrap_or(AlignContent::Stretch)
    }

    /// Parse the `flex` shorthand once (when present) into its components.
    fn flex_shorthand(&self) -> Option<FlexShorthand> {
        self.0.get("flex").and_then(|v| parse_flex_shorthand(v))
    }

    /// `flex-grow` (defaults to `0`). Falls back to the `flex` shorthand's
    /// grow component when `flex-grow` is unset.
    pub fn get_flex_grow(&self) -> f32 {
        match self.0.get("flex-grow") {
            Some(v) => parse_non_negative_number(v).unwrap_or(0.0),
            None => self.flex_shorthand().map(|f| f.grow).unwrap_or(0.0),
        }
    }

    /// `flex-shrink` (defaults to `1`). Falls back to the `flex` shorthand's
    /// shrink component when `flex-shrink` is unset.
    pub fn get_flex_shrink(&self) -> f32 {
        match self.0.get("flex-shrink") {
            Some(v) => parse_non_negative_number(v).unwrap_or(1.0),
            None => self.flex_shorthand().map(|f| f.shrink).unwrap_or(1.0),
        }
    }

    /// `flex-basis` (defaults to `Auto`). Falls back to the `flex` shorthand's
    /// basis component when `flex-basis` is unset.
    pub fn get_flex_basis(&self) -> FlexBasis {
        match self.0.get("flex-basis") {
            Some(v) => parse_flex_basis(v).unwrap_or(FlexBasis::Auto),
            None => self
                .flex_shorthand()
                .map(|f| f.basis)
                .unwrap_or(FlexBasis::Auto),
        }
    }

    /// Resolve `flex-basis` against the containing main-axis size.
    ///
    /// Returns `None` for `Auto` so the caller can apply its own fallback
    /// chain (`width` / `height` → content measure → equal share). A
    /// percentage basis against an indefinite containing main size is also
    /// `None` (CSS: percentages against an indefinite axis compute to
    /// `auto`).
    pub fn get_flex_basis_resolved(
        &self,
        containing_main: Option<usize>,
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

    /// Resolve the multi-column properties (`columns` / `column-count` /
    /// `column-width` / `column-gap` / `column-fill` / `column-rule`) into a
    /// concrete [`ColumnsResolved`] spec.
    ///
    /// `available` is the content-box width the columns share. Follows the CSS
    /// Multi-column pseudo-algorithm: an explicit `column-count` caps the
    /// number of columns; an explicit `column-width` limits it to how many
    /// such columns (plus `gap`) fit; when both are given the smaller wins;
    /// when only `column-width` is given the count is derived from the
    /// available width. The used column width always widens to fill the
    /// container evenly. Returns `None` when no column property is declared or
    /// the resolved count is 1 (no fragmentation).
    pub fn get_columns_resolved(
        &self,
        available: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<ColumnsResolved> {
        // `column-gap` defaults to `normal` == 1em in browsers; in cells that
        // is one font-size unit. Percentages resolve against the available
        // width (the multicol container is in scope here, unlike the flex
        // `gap` where the axis size is not yet known).
        let gap = self
            .0
            .get("column-gap")
            .and_then(|v| Length::parse(v))
            .map(|l| {
                l.resolve(LengthContext {
                    containing: available,
                    font: font_ctx,
                    viewport,
                })
            })
            .unwrap_or(font_ctx.font_size_cells);

        let shorthand = self
            .0
            .get("columns")
            .and_then(|v| parse_columns_shorthand(v));
        let count_decl = self
            .0
            .get("column-count")
            .and_then(|v| parse_column_count(v))
            .or_else(|| shorthand.and_then(|s| s.count));
        let width_len = self
            .0
            .get("column-width")
            .and_then(|v| Length::parse(v))
            .or_else(|| shorthand.and_then(|s| s.width))
            .filter(|l| !matches!(l, Length::Auto));

        if count_decl.is_none() && width_len.is_none() {
            return None;
        }

        let width_cells = width_len.map(|l| {
            l.resolve(LengthContext {
                containing: available,
                font: font_ctx,
                viewport,
            })
        });
        let max_by_width = width_cells.map(|w| {
            let unit = w.saturating_add(gap).max(1);
            (available.saturating_add(gap) / unit).max(1)
        });

        let count = match (count_decl, max_by_width) {
            (Some(c), Some(m)) => c.min(m),
            (Some(c), None) => c,
            (None, Some(m)) => m,
            (None, None) => return None,
        };
        if count <= 1 {
            return None;
        }

        // Shrink the count until every column keeps at least one cell; on a
        // very narrow container an eager `column-count` would otherwise leave
        // zero-width columns.
        let mut count = count;
        while count > 1
            && available
                < count
                    .saturating_sub(1)
                    .saturating_mul(gap)
                    .saturating_add(count)
        {
            count -= 1;
        }
        if count <= 1 {
            return None;
        }
        let col_width =
            available.saturating_sub(count.saturating_sub(1).saturating_mul(gap)) / count;

        Some(ColumnsResolved {
            count,
            col_width: col_width.max(1),
            gap,
            fill: self.parse_or("column-fill", parse_column_fill, ColumnFill::default()),
            rule: self.get_column_rule(available, font_ctx, viewport),
        })
    }

    /// Resolve `column-rule` — shorthand plus `column-rule-width` /
    /// `column-rule-style` / `column-rule-color` longhands (longhands win,
    /// mirroring the border precedence). Returns `None` when the style is
    /// `none`, so no rule is painted.
    fn get_column_rule(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<Border> {
        let shorthand = self
            .0
            .get("column-rule")
            .and_then(|v| parse_column_rule(v, containing_width, font_ctx, viewport));
        let width_declared = self.0.contains_key("column-rule-width");
        let mut width = self
            .0
            .get("column-rule-width")
            .map(|v| parse_border_width(Some(v.as_ref()), containing_width, font_ctx, viewport))
            .or_else(|| shorthand.map(|b| b.width))
            .unwrap_or(0);
        let style = self
            .0
            .get("column-rule-style")
            .map(|v| parse_border_style(Some(v.as_ref())))
            .or_else(|| shorthand.map(|b| b.style))
            .unwrap_or(BorderStyle::None);
        let color = self
            .0
            .get("column-rule-color")
            .and_then(|v| parse_color_str(v))
            .or_else(|| shorthand.and_then(|b| b.color));
        if !width_declared && width == 0 && style == BorderStyle::Solid {
            // `column-rule: solid red` names no width — default to one cell so
            // the rule is visible (browsers default to `medium`).
            width = 1;
        }
        (style != BorderStyle::None).then_some(Border::new(width, style, color))
    }

    /// Element height (CSS `height`).
    ///
    /// Mirrors [`get_width`](Self::get_width) for the cross-axis size. Used
    /// by the flex layout to detect an explicit container height so
    /// `align-items: stretch` can take effect (stretch is otherwise a no-op
    /// in v1 — see `flex::tree` for the rationale).
    ///
    /// `containing_height` is the containing block's *height* when it is
    /// definite, `None` when indefinite (normal flow). Percentages resolve
    /// against that height — and with an indefinite containing height they
    /// compute to `auto` (CSS 10.5), never against the width.
    pub fn get_height(
        &self,
        containing_height: Option<usize>,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        let raw = self.0.get("height")?;
        let len = Length::parse(raw)?;
        match (len, containing_height) {
            (Length::Auto, _) => None,
            (Length::Pct(_), None) => None,
            (len, containing) => Some(len.resolve(LengthContext {
                containing: containing.unwrap_or(0),
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
        self.resolve_min_max_len("min-width", Some(containing_width), font_ctx, viewport)
    }

    /// Maximum width (CSS `max-width`). `None` when unset / `none` / `auto`.
    pub fn get_max_width(
        &self,
        containing_width: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        self.resolve_min_max_len("max-width", Some(containing_width), font_ctx, viewport)
    }

    /// Minimum height (CSS `min-height`). `None` when unset / `auto`, or
    /// when a percentage meets an indefinite containing height.
    pub fn get_min_height(
        &self,
        containing_height: Option<usize>,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        self.resolve_min_max_len("min-height", containing_height, font_ctx, viewport)
    }

    /// Maximum height (CSS `max-height`). `None` when unset / `none` / `auto`,
    /// or when a percentage meets an indefinite containing height.
    pub fn get_max_height(
        &self,
        containing_height: Option<usize>,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        self.resolve_min_max_len("max-height", containing_height, font_ctx, viewport)
    }

    /// Shared resolution for the four min/max size longhands.
    ///
    /// `auto` (or `none`, which fails [`Length::parse`]) means "no
    /// constraint" — the property's initial value — so both map to `None`
    /// and the caller leaves the size unconstrained. `containing_size` is
    /// the containing block's size **along the same axis as the property**
    /// (width for `min/max-width`, height for `min/max-height`); a
    /// percentage against an indefinite (`None`) axis computes to `None`.
    fn resolve_min_max_len(
        &self,
        key: &str,
        containing_size: Option<usize>,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> Option<usize> {
        let raw = self.0.get(key)?;
        let len = Length::parse(raw)?;
        match (len, containing_size) {
            (Length::Auto, _) => None,
            (Length::Pct(_), None) => None,
            (len, containing) => Some(len.resolve(LengthContext {
                containing: containing.unwrap_or(0),
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
            self.0
                .get(key)
                .and_then(|raw| match Length::parse_signed(raw)? {
                    Length::Auto => None,
                    len => Some(len.resolve_signed(LengthContext {
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

    /// List marker style (CSS `list-style-type`).
    ///
    /// Inherited, so a `list-style-type` on the `<ul>` / `<ol>` reaches the
    /// `<li>` markers through the cascade. The default is element-driven:
    /// `disc` under `<ul>` / `<menu>`, `decimal` under `<ol>` — the caller
    /// (the `<li>` layout) supplies the element default, mirroring
    /// [`Self::get_white_space`]'s UA-default pattern.
    pub fn get_list_style_type(&self, default: ListStyleType) -> ListStyleType {
        self.parse_or("list-style-type", parse_list_style_type, default)
    }

    /// Table column-sizing algorithm (CSS `table-layout`).
    pub fn get_table_layout(&self) -> TableLayout {
        self.parse_or("table-layout", parse_table_layout, TableLayout::default())
    }

    /// Letter spacing in cells (CSS `letter-spacing`) — added after every
    /// grapheme of the node's inline runs. Negative values clamp to 0
    /// (terminal cells cannot overlap).
    pub fn get_letter_spacing(&self, font_ctx: FontContext, viewport: Viewport) -> usize {
        self.0
            .get("letter-spacing")
            .and_then(|raw| Length::parse(raw))
            .map(|len| {
                len.resolve(LengthContext {
                    containing: 0,
                    font: font_ctx,
                    viewport,
                })
            })
            .unwrap_or(0)
    }

    /// Extra spacing inserted at each word boundary (CSS `word-spacing`) —
    /// added to every space grapheme. Signed: a negative value tightens
    /// words (each space shrinks, floored at a zero-width join).
    pub fn get_word_spacing(&self, font_ctx: FontContext, viewport: Viewport) -> isize {
        self.0
            .get("word-spacing")
            .and_then(|raw| Length::parse(raw))
            .map(|len| {
                len.resolve(LengthContext {
                    containing: 0,
                    font: font_ctx,
                    viewport,
                }) as isize
            })
            .unwrap_or(0)
    }

    /// First-line indent in cells (CSS `text-indent`) — narrows the first
    /// visual line's wrap width and shifts its paint origin right.
    pub fn get_text_indent(
        &self,
        containing: usize,
        font_ctx: FontContext,
        viewport: Viewport,
    ) -> usize {
        self.0
            .get("text-indent")
            .and_then(|raw| Length::parse(raw))
            .map(|len| {
                len.resolve(LengthContext {
                    containing,
                    font: font_ctx,
                    viewport,
                })
            })
            .unwrap_or(0)
    }

    /// Clipped-line ending (CSS `text-overflow`).
    pub fn get_text_overflow(&self) -> crate::html::css_parse::TextOverflow {
        self.parse_or(
            "text-overflow",
            crate::html::css_parse::parse_text_overflow,
            crate::html::css_parse::TextOverflow::default(),
        )
    }

    /// Which side of the table the `<caption>` row renders on
    /// (CSS `caption-side`).
    pub fn get_caption_side(&self) -> CaptionSide {
        self.parse_or("caption-side", parse_caption_side, CaptionSide::default())
    }

    /// Whether cells without visible content paint their chrome
    /// (CSS `empty-cells`).
    pub fn get_empty_cells(&self) -> EmptyCells {
        self.parse_or("empty-cells", parse_empty_cells, EmptyCells::default())
    }
}

/// Clamp a resolved axis size into `[min, max]`.
///
/// The single canonical min/max clamp for the layout engine (blocks, flex
/// items and replaced controls all funnel through here). When `min`
/// exceeds `max`, the minimum wins — CSS resolves conflicting min/max
/// constraints in favour of the minimum.
pub(crate) fn clamp_min_max(value: usize, min: Option<usize>, max: Option<usize>) -> usize {
    let mut clamped = value;
    if let Some(max) = max {
        clamped = clamped.min(max);
    }
    if let Some(min) = min {
        clamped = clamped.max(min);
    }
    clamped
}

use crossterm::style::Color;
use indexmap::IndexMap;

use crate::html::box_edges::{Border, BorderSides, BoxEdges, EdgeSizes};

use super::values::{FontStyleKind, FontWeight, Length, TextAlign, TextDecoration, parse_border_side, parse_border_style, parse_border_width, parse_color_str, parse_edge_sizes};

#[derive(Debug, Clone, Default)]
pub struct CssAttr(pub IndexMap<String, String>);

impl CssAttr {
    pub fn new() -> Self {
        CssAttr(IndexMap::new())
    }
    pub fn add(&mut self, css: CssAttr) {
        self.0.extend(css.0);
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
    ///
    /// `em` / `rem` units are multiplied by `font_size_cells` /
    /// `root_font_size_cells` so they reflect the relative size specified by
    /// CSS. Floats (`1.5em`, `33.33%`) are rounded to the nearest integer
    /// cell — terminals cannot render half columns. See [`Length::parse`]
    /// for the underlying value type.
    pub fn get_width(
        &self,
        term_columns: usize,
        font_size_cells: usize,
        root_font_size_cells: usize,
    ) -> Option<usize> {
        let raw = self.0.get("width")?;
        match Length::parse(raw)? {
            Length::Auto => None,
            len => Some(len.resolve(term_columns, font_size_cells, root_font_size_cells)),
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
    /// font-size.
    pub fn get_box_edges(
        &self,
        containing_width: usize,
        font_size_cells: usize,
        root_font_size_cells: usize,
    ) -> BoxEdges {
        BoxEdges {
            padding: self.get_padding(containing_width, font_size_cells, root_font_size_cells),
            border: self.get_border(containing_width, font_size_cells, root_font_size_cells),
            margin: self.get_margin(containing_width, font_size_cells, root_font_size_cells),
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
    /// element's font.
    pub fn get_padding(
        &self,
        containing_width: usize,
        font_size_cells: usize,
        root_font_size_cells: usize,
    ) -> EdgeSizes {
        parse_edge_sizes(
            &self.0,
            "padding",
            containing_width,
            font_size_cells,
            root_font_size_cells,
        )
    }

    /// Resolve all four margin edges from `margin` / `margin-{side}`.
    ///
    /// `margin: auto` is currently treated as `0` (the terminal has no
    /// centering primitive for margins). Per-side longhands win over the
    /// shorthand.
    pub fn get_margin(
        &self,
        containing_width: usize,
        font_size_cells: usize,
        root_font_size_cells: usize,
    ) -> EdgeSizes {
        parse_edge_sizes(
            &self.0,
            "margin",
            containing_width,
            font_size_cells,
            root_font_size_cells,
        )
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
        font_size_cells: usize,
        root_font_size_cells: usize,
    ) -> BorderSides {
        // Step 1: collect universal values (lowest priority).
        let universal_width = parse_border_width(
            self.0.get("border-width").map(|s| s.as_str()),
            containing_width,
            font_size_cells,
            root_font_size_cells,
        );
        let universal_style =
            parse_border_style(self.0.get("border-style").map(|s| s.as_str()));
        let universal_color = self
            .0
            .get("border-color")
            .and_then(|s| parse_color_str(s));

        let universal = Border::new(universal_width, universal_style, universal_color);

        // Step 2: parse per-side longhands.
        let top = parse_border_side(
            self.0.get("border-top"),
            self.0.get("border"),
            universal,
            containing_width,
            font_size_cells,
            root_font_size_cells,
        );
        let right = parse_border_side(
            self.0.get("border-right"),
            self.0.get("border"),
            universal,
            containing_width,
            font_size_cells,
            root_font_size_cells,
        );
        let bottom = parse_border_side(
            self.0.get("border-bottom"),
            self.0.get("border"),
            universal,
            containing_width,
            font_size_cells,
            root_font_size_cells,
        );
        let left = parse_border_side(
            self.0.get("border-left"),
            self.0.get("border"),
            universal,
            containing_width,
            font_size_cells,
            root_font_size_cells,
        );

        BorderSides {
            top,
            right,
            bottom,
            left,
        }
    }
}

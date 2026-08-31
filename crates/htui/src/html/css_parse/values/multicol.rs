use super::length::{FontContext, Length, Viewport};
use super::parse_border_shorthand;

use crate::html::box_edges::Border;

/// CSS `column-fill` value (Multi-column Layout).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColumnFill {
    /// `balance` (default) — columns get an equal share of the content's
    /// height.
    #[default]
    Balance,
    /// `auto` — columns are filled in order; later columns hold whatever is
    /// left over.
    Auto,
}

/// Parse a `column-fill` keyword.
#[doc(hidden)]
pub fn parse_column_fill(s: &str) -> Option<ColumnFill> {
    match s.trim().to_ascii_lowercase().as_str() {
        "balance" | "balance-all" => Some(ColumnFill::Balance),
        "auto" => Some(ColumnFill::Auto),
        _ => None,
    }
}

/// Parse a `column-count` integer (1..=10000).
#[doc(hidden)]
pub fn parse_column_count(s: &str) -> Option<usize> {
    let n = s.trim().parse::<usize>().ok()?;
    (1..=10000).contains(&n).then_some(n)
}

/// The `columns` shorthand components: `<'column-width'> || <'column-count'>`
/// in any order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnsShorthand {
    /// The `column-width` component (`None` when absent).
    pub width: Option<Length>,
    /// The `column-count` component (`None` when absent).
    pub count: Option<usize>,
}

/// Parse the `columns` shorthand (`12em 3` / `3` / `12em`). Each token is
/// classified as either an integer (count) or a length (width); a duplicate
/// or unrecognised token rejects the whole declaration.
#[doc(hidden)]
pub fn parse_columns_shorthand(s: &str) -> Option<ColumnsShorthand> {
    let mut width: Option<Length> = None;
    let mut count: Option<usize> = None;
    for token in s.split_whitespace() {
        if let Some(c) = parse_column_count(token) {
            if count.is_some() {
                return None;
            }
            count = Some(c);
            continue;
        }
        let l = Length::parse(token)?;
        if width.is_some() {
            return None;
        }
        width = Some(l);
    }
    (width.is_some() || count.is_some()).then_some(ColumnsShorthand { width, count })
}

/// Parse a `column-rule` shorthand (`1px solid red`, `solid red`, …) into a
/// [`Border`]. Reuses the border token classifier so widths / styles / colors
/// may appear in any order. Returns `None` when nothing recognizable is
/// present.
#[doc(hidden)]
pub fn parse_column_rule(
    s: &str,
    containing_width: usize,
    font_ctx: FontContext,
    viewport: Viewport,
) -> Option<Border> {
    parse_border_shorthand(s, containing_width, font_ctx, viewport)
}

/// Fully resolved multi-column layout.
///
/// Produced by [`CssAttr::get_columns_resolved`](crate::html::css_parse::CssAttr)
/// and consumed by the block layout to fragment a box's content into
/// side-by-side columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnsResolved {
    /// Number of columns (`>= 2` — single-column layouts never resolve here).
    pub count: usize,
    /// Content width of one column in cells.
    pub col_width: usize,
    /// Gap between adjacent columns in cells.
    pub gap: usize,
    /// `column-fill` strategy (`balance` by default).
    pub fill: ColumnFill,
    /// `column-rule` (visible when its style is not `none`).
    pub rule: Option<Border>,
}

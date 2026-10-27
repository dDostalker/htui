//! `@media` query parsing and viewport matching.
//!
//! Supported subset: the four size features (`min-width` / `max-width` /
//! `min-height` / `max-height`), `and` between features inside one query, and
//! comma-separated queries (OR). Feature values are lengths interpreted as
//! **terminal cells**: a bare number and `ch` are both cell counts (a
//! terminal has no CSS pixel; the media query is written against the cell
//! grid, so `(max-height: 14)` means "14 rows or fewer"). Other units
//! (`em` / `rem` / `%` / `vw` / `vh`) are rejected with a warning — the whole
//! query then never matches, mirroring the CSS rule that an unknown media
//! feature makes its query false.

use log::warn;

use super::values::{Length, Viewport};

/// One `@media` query: every present feature must hold (AND) for the query
/// to match. A missing feature imposes no constraint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaQuery {
    pub max_width: Option<f32>,
    pub min_width: Option<f32>,
    pub max_height: Option<f32>,
    pub min_height: Option<f32>,
}

/// A comma-separated query list: matches when **any** query matches (CSS OR).
/// An empty list never matches.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaQueryList(pub Vec<MediaQuery>);

impl MediaQueryList {
    /// Whether any query in the list matches `viewport`.
    ///
    /// The default viewport (`0x0`) stands for "no viewport known" — a real
    /// terminal is at least one cell in each axis — and size conditions never
    /// match it, so unconditional styling is unaffected by its presence.
    pub fn matches(&self, viewport: &Viewport) -> bool {
        viewport.cols > 0
            && viewport.rows > 0
            && self
                .0
                .iter()
                .any(|q| q.matches(viewport.cols as f32, viewport.rows as f32))
    }
}

impl MediaQuery {
    /// AND of every present feature against the cell counts `(cols, rows)`.
    /// The comparison is inclusive: a `cols`-wide viewport matches
    /// `(max-width: cols)`.
    fn matches(&self, cols: f32, rows: f32) -> bool {
        let fits = |v: f32, max: Option<f32>, min: Option<f32>| {
            max.is_none_or(|m| v <= m) && min.is_none_or(|m| v >= m)
        };
        fits(cols, self.max_width, self.min_width) && fits(rows, self.max_height, self.min_height)
    }
}

/// Parse a `@media` prelude (`(max-width: 70ch) and (min-height: 10),
/// (max-width: 40)`) into a query list.
///
/// Queries that fail to parse are dropped with a warning; if *every* query is
/// dropped the list is empty and matches nothing — the block's rules survive
/// parsing but never apply (CSS treats an unknown media query as false). Only
/// a blank prelude returns `None`, which drops the whole block.
pub(crate) fn parse_media_query_list(text: &str) -> Option<MediaQueryList> {
    let mut queries = Vec::new();
    for part in split_top_level_commas(text) {
        match parse_one_query(part) {
            Some(query) => queries.push(query),
            None => warn!("Ignoring unsupported @media query: {part}"),
        }
    }
    if text.trim().is_empty() {
        None
    } else {
        Some(MediaQueryList(queries))
    }
}

/// Parse one comma-free query — the `and`-separated feature list
/// (`(max-width: 70ch)` or `(max-width: 70ch) and (max-height: 14)`).
fn parse_one_query(text: &str) -> Option<MediaQuery> {
    let mut query = MediaQuery {
        max_width: None,
        min_width: None,
        max_height: None,
        min_height: None,
    };
    // `and` is case-insensitive per the CSS grammar; splitting on the word
    // boundary keeps `(max-width: 70ch)` intact.
    let mut features = Vec::new();
    let mut rest = text.trim();
    while let Some(idx) = find_and_keyword(rest) {
        features.push(&rest[..idx]);
        rest = rest[idx + 3..].trim();
    }
    features.push(rest);
    for feature in features {
        parse_feature(feature, &mut query)?;
    }
    Some(query)
}

/// Find the next standalone `and` keyword (case-insensitive), outside any
/// parentheses. Returns its byte offset, or `None`.
fn find_and_keyword(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i + 3 <= bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'a' | b'A' if depth == 0 => {
                let word = &text[i..i + 3];
                if word.eq_ignore_ascii_case("and") {
                    let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
                    let after = bytes.get(i + 3);
                    let after_ok = after.is_none_or(|b| !b.is_ascii_alphanumeric());
                    if before_ok && after_ok {
                        return Some(i);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Parse one `(feature: value)` group into `query`. Returns `None` on an
/// unknown feature or an unsupported value unit (the whole query is dropped).
fn parse_feature(text: &str, query: &mut MediaQuery) -> Option<()> {
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?.trim();
    let (name, value) = inner.split_once(':')?;
    let name = name.trim().to_ascii_lowercase();
    let cells = feature_value_cells(value.trim())?;
    let slot = match name.as_str() {
        "max-width" => &mut query.max_width,
        "min-width" => &mut query.min_width,
        "max-height" => &mut query.max_height,
        "min-height" => &mut query.min_height,
        _ => {
            warn!("Unsupported @media feature: {name}");
            return None;
        }
    };
    // A repeated feature narrows its own constraint like two separate
    // features would (`and` of both bounds).
    *slot = Some(match *slot {
        Some(prev) if name.starts_with("max-") => prev.min(cells),
        Some(prev) => prev.max(cells),
        None => cells,
    });
    Some(())
}

/// Interpret a media-feature value as a terminal cell count: a bare number
/// and `ch` are both cell counts; any other unit is rejected.
fn feature_value_cells(text: &str) -> Option<f32> {
    match Length::parse(text) {
        Some(Length::Px(n)) | Some(Length::Ch(n)) => Some(n),
        Some(other) => {
            warn!("Unsupported @media value unit: {other:?}");
            None
        }
        None => {
            warn!("Invalid @media value: {text}");
            None
        }
    }
}

/// Split a comma-separated query list at top-level commas only (commas inside
/// parentheses — there are none in the supported feature grammar, but a
/// malformed query must not corrupt the split). Trimmed; empties dropped.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut last = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = s[last..i].trim();
                if !part.is_empty() {
                    out.push(part);
                }
                last = i + 1;
            }
            _ => {}
        }
    }
    if let Some(tail) = s.get(last..) {
        let tail = tail.trim();
        if !tail.is_empty() {
            out.push(tail);
        }
    }
    out
}

//! `@media` query parsing and viewport matching.
//!
//! Supported subset: the four size features (`min-width` / `max-width` /
//! `min-height` / `max-height`), `(orientation: landscape | portrait)`, media
//! types (`screen` / `all` match, `print` / `speech` never do), `not` /
//! `only` prefixes, `and` between features inside one query, and
//! comma-separated queries (OR). Feature values are lengths interpreted as
//! **terminal cells**: a bare number and `ch` are both cell counts (a
//! terminal has no CSS pixel; the media query is written against the cell
//! grid, so `(max-height: 14)` means "14 rows or fewer"). Other units
//! (`em` / `rem` / `%` / `vw` / `vh`) are rejected with a warning — the whole
//! query then never matches, mirroring the CSS rule that an unknown media
//! feature makes its query false. Nested `@media` and the modern range
//! syntax (`(width > 40ch)`) are not supported.

use log::warn;

use super::values::{Length, Viewport};

/// Terminal orientation: the CSS `landscape` / `portrait` keywords mapped to
/// the cell grid (a terminal can legitimately be taller than it is wide).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// `cols >= rows`.
    Landscape,
    /// `rows > cols`.
    Portrait,
}

/// One `@media` query: the (optional) media type plus every present feature
/// must hold (AND) for the query to match; `negated` inverts the whole
/// result. A missing feature imposes no constraint.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediaQuery {
    pub negated: bool,
    /// Media type gate (`None` = any type matches). `screen` / `all` match;
    /// `print` / `speech` never do.
    pub media_type: Option<bool>,
    pub max_width: Option<f32>,
    pub min_width: Option<f32>,
    pub max_height: Option<f32>,
    pub min_height: Option<f32>,
    pub orientation: Option<Orientation>,
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
    /// AND of the media type, every present feature and the `not` inversion,
    /// against the cell counts `(cols, rows)`. The size comparison is
    /// inclusive: a `cols`-wide viewport matches `(max-width: cols)`.
    fn matches(&self, cols: f32, rows: f32) -> bool {
        let fits = |v: f32, max: Option<f32>, min: Option<f32>| {
            max.is_none_or(|m| v <= m) && min.is_none_or(|m| v >= m)
        };
        let orientation_ok = match self.orientation {
            Some(Orientation::Landscape) => cols >= rows,
            Some(Orientation::Portrait) => rows > cols,
            None => true,
        };
        let result = self.media_type.unwrap_or(true)
            && orientation_ok
            && fits(cols, self.max_width, self.min_width)
            && fits(rows, self.max_height, self.min_height);
        result != self.negated
    }
}

/// Parse a `@media` prelude (`screen and (max-width: 70ch), not (min-height:
/// 10)`) into a query list.
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

/// Parse one comma-free query — `[not|only] [type] [and (feature)...]`.
fn parse_one_query(text: &str) -> Option<MediaQuery> {
    let mut query = MediaQuery {
        negated: false,
        media_type: None,
        max_width: None,
        min_width: None,
        max_height: None,
        min_height: None,
        orientation: None,
    };
    let mut rest = text.trim();
    // `only` is a legacy prefix with no matching semantics; `not` inverts the
    // whole query. Queries may also start directly with the feature list.
    loop {
        rest = rest.trim();
        let Some((word, tail)) = leading_ident(rest) else {
            break;
        };
        match word.to_ascii_lowercase().as_str() {
            "only" => rest = tail,
            "not" => {
                query.negated = !query.negated;
                rest = tail;
            }
            _ => break,
        }
    }
    // A leading bare identifier is the media type (only valid before the
    // first `and`-joined feature list).
    rest = rest.trim();
    let (word, tail) = leading_ident(rest).unwrap_or_default();
    match word.to_ascii_lowercase().as_str() {
        "" => {}
        "screen" | "all" => {
            query.media_type = Some(true);
            rest = tail;
        }
        "print" | "speech" => {
            query.media_type = Some(false);
            rest = tail;
        }
        _ => {
            warn!("Unsupported @media type: {word}");
            return None;
        }
    }
    // `and`-separated feature groups: the first group follows directly (no
    // `and`), every further group is prefixed by `and`.
    loop {
        rest = rest.trim();
        if rest.is_empty() {
            break;
        }
        match leading_ident(rest) {
            Some((word, tail)) => {
                if !word.eq_ignore_ascii_case("and") {
                    return None;
                }
                let (feature_text, remainder) = parenthesized_group(tail)?;
                parse_feature(feature_text, &mut query)?;
                rest = remainder;
            }
            None => {
                let (feature_text, remainder) = parenthesized_group(rest)?;
                parse_feature(feature_text, &mut query)?;
                rest = remainder;
            }
        }
    }
    Some(query)
}

/// Split a leading ASCII identifier off `text`, returning `(ident, rest)`.
/// `None` when `text` does not start with an identifier character.
fn leading_ident(text: &str) -> Option<(&str, &str)> {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    Some((&text[..end], &text[end..]))
}

/// Split a leading `( … )` group off `text`, returning `(inner, rest)`.
fn parenthesized_group(text: &str) -> Option<(&str, &str)> {
    let text = text.trim();
    let inner = text.strip_prefix('(')?;
    let mut depth = 1usize;
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&inner[..i], &inner[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse one `(feature: value)` group into `query`. Returns `None` on an
/// unknown feature or an unsupported value unit (the whole query is dropped).
fn parse_feature(text: &str, query: &mut MediaQuery) -> Option<()> {
    let inner = text.trim();
    let (name, value) = if let Some((n, v)) = inner.split_once(':') {
        (n.trim().to_ascii_lowercase(), Some(v.trim()))
    } else {
        (inner.to_ascii_lowercase(), None)
    };
    match name.as_str() {
        "max-width" | "min-width" | "max-height" | "min-height" => {
            let cells = feature_value_cells(value?)?;
            let slot = match name.as_str() {
                "max-width" => &mut query.max_width,
                "min-width" => &mut query.min_width,
                "max-height" => &mut query.max_height,
                _ => &mut query.min_height,
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
        "orientation" => {
            let keyword = value?.to_ascii_lowercase();
            let orientation = match keyword.as_str() {
                "landscape" => Orientation::Landscape,
                "portrait" => Orientation::Portrait,
                _ => {
                    warn!("Unsupported @media orientation: {keyword}");
                    return None;
                }
            };
            // A repeated orientation cannot hold both ways — the query is
            // contradictory and never matches.
            if query.orientation.is_some_and(|o| o != orientation) {
                return None;
            }
            query.orientation = Some(orientation);
            Some(())
        }
        _ => {
            warn!("Unsupported @media feature: {name}");
            None
        }
    }
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

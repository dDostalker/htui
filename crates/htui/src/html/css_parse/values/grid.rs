use std::sync::Arc;

use super::length::Length;

// ============================================================
// CSS Grid values
//
// A deliberately small, typed subset of CSS Grid.  Track lists are
// parsed eagerly (integer `repeat()` counts expand here, while
// `repeat(auto-fill | auto-fit, …)` is kept for the layout to resolve
// against the definite container size), and line placements remain raw
// line numbers/spans until the grid algorithm knows the final track
// count (needed for negative line numbers).  A bare custom-ident names
// a `grid-template-areas` area and resolves to its edge line at layout
// time; `grid-template-areas` itself is validated into a rectangular
// cell matrix right here.
// ============================================================

/// One track size in `grid-template-columns` / `grid-template-rows`.
#[derive(Debug, Clone, PartialEq)]
pub enum GridTrack {
    /// `auto`: size to the track's content.
    Auto,
    /// A resolved CSS length or percentage.
    Length(Length),
    /// `<flex>`: share the remaining space after fixed and auto tracks.
    Fr(f32),
    /// `minmax(min, max)`: the base size comes from the `min` side (a
    /// content hint or length), free space then grows the track up to the
    /// `max` side before `fr` distribution. `fit-content(x)` parses as
    /// `minmax(auto, x)`.
    MinMax {
        min: Box<GridTrack>,
        max: Box<GridTrack>,
    },
    /// `repeat(auto-fill | auto-fit, tracks)`: the repetition count is
    /// resolved at layout time from the definite container size, counting
    /// each track by its definite width (its max if definite, else its min).
    /// Only legal in the explicit `grid-template-*` lists; nested
    /// auto-repeats are rejected.
    AutoRepeat {
        /// `true` for `auto-fit` (empty trailing tracks collapse), `false`
        /// for `auto-fill` (every repetition is kept).
        fit: bool,
        tracks: Vec<GridTrack>,
    },
}

impl GridTrack {
    /// Parse a single track size. `min-content` and `max-content` are
    /// accepted as content-sized aliases for `auto` on the terminal model.
    pub fn parse(value: &str) -> Option<Self> {
        let s = value.trim();
        if s.eq_ignore_ascii_case("auto")
            || s.eq_ignore_ascii_case("min-content")
            || s.eq_ignore_ascii_case("max-content")
        {
            return Some(GridTrack::Auto);
        }
        if let Some(raw) = s.strip_suffix("fr") {
            let n = raw.trim().parse::<f32>().ok()?;
            return (!n.is_nan() && n >= 0.0).then_some(GridTrack::Fr(n));
        }
        if let Some(inner) = function_args(s, "minmax") {
            let mut parts = split_top_level(inner, ',')?;
            if parts.len() != 2 {
                return None;
            }
            let max = parts.pop()?;
            let min = parts.pop()?;
            let min = GridTrack::parse(&min)?;
            let max = GridTrack::parse(&max)?;
            // A track breadth is never flexible on the min side and never a
            // nested function on either side.
            if matches!(min, GridTrack::Fr(_) | GridTrack::MinMax { .. })
                || matches!(max, GridTrack::MinMax { .. })
            {
                return None;
            }
            return Some(GridTrack::MinMax {
                min: Box::new(min),
                max: Box::new(max),
            });
        }
        if let Some(inner) = function_args(s, "fit-content") {
            let length = Length::parse(inner.trim())?;
            return Some(GridTrack::MinMax {
                min: Box::new(GridTrack::Auto),
                max: Box::new(GridTrack::Length(length)),
            });
        }
        Length::parse(s).map(GridTrack::Length)
    }

    /// Whether this track participates in flex (fr) distribution.
    pub fn is_fr(self) -> bool {
        match self {
            GridTrack::Fr(_) => true,
            GridTrack::MinMax { max, .. } => matches!(*max, GridTrack::Fr(_)),
            _ => false,
        }
    }
}

/// Slice the argument list out of a case-insensitive `name(...)` call.
fn function_args<'a>(token: &'a str, name: &str) -> Option<&'a str> {
    let token = token.trim();
    let lower = token.to_ascii_lowercase();
    let open = format!("{name}(");
    let inner = lower.strip_prefix(&open)?.strip_suffix(')')?;
    // ASCII-only lowercasing keeps byte offsets valid in the original.
    Some(&token[open.len()..open.len() + inner.len()])
}

/// Explicit template tracks and the track used for implicit tracks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridTemplate {
    /// Explicit `grid-template-columns` tracks.
    pub columns: Vec<GridTrack>,
    /// Explicit `grid-template-rows` tracks.
    pub rows: Vec<GridTrack>,
}

/// Parse a whitespace-separated track list, expanding `repeat(N, ...)`.
///
/// The terminal grid model does not accept named lines or named-area
/// strings; returning `None` makes the CSS parser drop the declaration
/// instead of silently approximating it.
pub fn parse_grid_track_list(value: &str) -> Option<Vec<GridTrack>> {
    let value = value.trim();
    if value.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for token in split_top_level_whitespace(value)? {
        match parse_repeat(&token) {
            Some(tracks) => out.extend(tracks),
            None => out.push(GridTrack::parse(&token)?),
        }
        if out.len() > 4096 {
            // Guard against pathological repeated track lists.
            return None;
        }
    }
    Some(out)
}

/// Parse a `grid-template` shorthand into column and row track lists.
///
/// The supported grammar is `<track-list> [/ <track-list>]`. Named-line
/// brackets and named-area strings are intentionally rejected here — the
/// areas form expands at declaration-parse time via
/// [`expand_grid_template`].
pub fn parse_grid_template(value: &str) -> Option<GridTemplate> {
    let parts = split_top_level(value, '/')?;
    if parts.len() > 2 {
        return None;
    }
    let columns = parse_grid_track_list(parts.first()?)?;
    let rows = match parts.get(1) {
        Some(rows) => parse_grid_track_list(rows)?,
        None => Vec::new(),
    };
    Some(GridTemplate { columns, rows })
}

/// Expand an areas-form `grid-template` shorthand into its longhand
/// declarations: `grid-template: "a a" 1fr "b b" 2ch / 2ch 1fr` becomes
/// `grid-template-areas`, `grid-template-rows` and `grid-template-columns`
/// values. Row sizes default to `auto` per area row. A value without
/// quoted strings is the plain `<track-list>` form and keeps the raw
/// shorthand path ([`parse_grid_template`]); malformed area forms also
/// return `None` so the raw path rejects them.
pub(crate) fn expand_grid_template(value: &str) -> Option<Vec<(&'static str, Arc<str>)>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let mut parts = split_top_level(value, '/')?;
    if parts.len() > 2 {
        return None;
    }
    let columns = if parts.len() == 2 {
        let columns = parts.pop()?.trim().to_string();
        Some(columns)
    } else {
        None
    };
    let before = parts.pop()?;
    let (strings, runs) = scan_template_rows(&before)?;
    if strings.is_empty() {
        // Plain track-list form: the raw shorthand path owns it.
        return None;
    }
    if !runs[0].trim().is_empty() {
        return None;
    }
    let areas = strings.join(" ");
    let rows = (1..=strings.len())
        .map(|i| {
            let size = runs.get(i).map(|run| run.trim()).unwrap_or("");
            if size.is_empty() { "auto" } else { size }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut pairs = vec![
        ("grid-template-areas", Arc::from(areas.as_str())),
        ("grid-template-rows", Arc::from(rows.as_str())),
    ];
    if let Some(columns) = columns.filter(|columns| !columns.is_empty()) {
        pairs.push(("grid-template-columns", Arc::from(columns.as_str())));
    }
    Some(pairs)
}

/// Split an areas+sizes prefix into its quoted strings (quotes kept, the
/// way `grid-template-areas` expects them) and the raw text runs between
/// them, leading run first. An unterminated string fails.
fn scan_template_rows(value: &str) -> Option<(Vec<String>, Vec<String>)> {
    let mut strings: Vec<String> = Vec::new();
    let mut runs: Vec<String> = vec![String::new()];
    let mut quote: Option<char> = None;
    let mut current = String::new();
    for c in value.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    current.push(c);
                    quote = None;
                    strings.push(std::mem::take(&mut current));
                    runs.push(String::new());
                } else {
                    current.push(c);
                }
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                current.push(c);
            }
            None => {
                runs.last_mut()?.push(c);
            }
        }
    }
    (quote.is_none() && current.is_empty()).then_some((strings, runs))
}

/// A parsed `grid-template-areas` matrix: one entry per quoted row string,
/// each cell an area name or a null (`.`) cell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GridAreas {
    pub cells: Vec<Vec<Option<Arc<str>>>>,
}

impl GridAreas {
    /// Number of area rows (one per quoted string).
    pub fn row_count(&self) -> usize {
        self.cells.len()
    }

    /// Number of area columns (every row is validated to share this width).
    pub fn column_count(&self) -> usize {
        self.cells.first().map(|row| row.len()).unwrap_or(0)
    }
}

/// Parse `grid-template-areas`.
///
/// The value is a list of quoted strings, each one row of whitespace-
/// separated cells. A cell of only `.` characters is a null cell. Every row
/// must have the same column count and each name must occupy a single
/// rectangle; otherwise the whole declaration is invalid (`None`). `none`
/// clears the areas.
pub fn parse_grid_template_areas(value: &str) -> Option<GridAreas> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.eq_ignore_ascii_case("none") {
        return Some(GridAreas { cells: Vec::new() });
    }
    let mut rows = Vec::new();
    for token in split_quoted_area_strings(value)? {
        let inner = &token[1..token.len() - 1];
        let mut row = Vec::new();
        for cell in inner.split_whitespace() {
            if cell.chars().all(|c| c == '.') {
                row.push(None);
            } else if is_css_ident(cell) {
                row.push(Some(Arc::from(cell)));
            } else {
                return None;
            }
        }
        if row.is_empty() {
            return None;
        }
        rows.push(row);
    }
    let width = rows.first()?.len();
    if rows.iter().any(|row| row.len() != width) {
        return None;
    }
    // Each name must fill exactly one rectangle of the matrix.
    let mut areas: Vec<(&str, usize, usize, usize, usize, usize)> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let Some(name) = cell.as_deref() else {
                continue;
            };
            if let Some(entry) = areas.iter_mut().find(|a| a.0 == name) {
                entry.1 = entry.1.min(r);
                entry.2 = entry.2.max(r);
                entry.3 = entry.3.min(c);
                entry.4 = entry.4.max(c);
                entry.5 += 1;
            } else {
                areas.push((name, r, r, c, c, 1));
            }
        }
    }
    let rectangular = areas
        .iter()
        .all(|(_, r0, r1, c0, c1, cells)| (r1 - r0 + 1) * (c1 - c0 + 1) == *cells);
    rectangular.then_some(GridAreas { cells: rows })
}

/// Split the value into quoted strings. Whitespace between strings is
/// skipped; anything else unquoted makes the declaration invalid.
fn split_quoted_area_strings(value: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in value.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    current.push(c);
                    out.push(std::mem::take(&mut current));
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                current.push(c);
            }
            None if c.is_whitespace() => {}
            None => return None,
        }
    }
    (quote.is_none() && current.is_empty() && !out.is_empty()).then_some(out)
}

/// A raw `<grid-line>` before the final track count is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridLine {
    Auto,
    /// CSS line number (1-based positive, negative counted from the end).
    Line(isize),
    Span(usize),
    /// `span <n> <area-name>`: an N-track span anchored at the named area's
    /// edge — a start edge ends at the area's first line, an end edge starts
    /// at the line after the area. Resolved against `grid-template-areas` at
    /// collection time; an unknown name degrades to [`GridLine::Span`].
    SpanName {
        span: usize,
        name: Arc<str>,
    },
    /// A bare custom-ident naming a `grid-template-areas` area; the layout
    /// resolves it to the area's start/end edge line per axis.
    Name(Arc<str>),
}

impl GridLine {
    /// Parse one start/end component. A missing component is `auto`.
    pub fn parse(value: &str) -> Option<Self> {
        let s = value.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("auto") {
            return Some(GridLine::Auto);
        }
        if let Some(rest) = s.strip_prefix("span").map(str::trim_start) {
            // `span N`, `span N name`, `span name` — a bare `span` is invalid.
            let mut parts = rest.split_whitespace();
            let first = parts.next()?;
            if let Ok(n) = first.parse::<usize>() {
                if n == 0 {
                    return None;
                }
                return match parts.next() {
                    None => Some(GridLine::Span(n)),
                    Some(name)
                        if parts.next().is_none()
                            && is_css_ident(name)
                            && !name.eq_ignore_ascii_case("span") =>
                    {
                        Some(GridLine::SpanName {
                            span: n,
                            name: Arc::from(name),
                        })
                    }
                    _ => None,
                };
            }
            return (parts.next().is_none() && is_css_ident(first)).then_some(GridLine::SpanName {
                span: 1,
                name: Arc::from(first),
            });
        }
        // A bare `span` keyword is not a valid grid-line component.
        if s.eq_ignore_ascii_case("span") {
            return None;
        }
        if let Ok(n) = s.parse::<isize>()
            && n != 0
        {
            return Some(GridLine::Line(n));
        }
        is_css_ident(s).then(|| GridLine::Name(Arc::from(s)))
    }
}

/// A restricted CSS custom-ident: letters/`_`/non-ASCII to start (or `-`
/// followed by one of those), then also digits and `-`. Escaped characters
/// are not accepted.
fn is_css_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let ident_start = |c: char| c.is_alphabetic() || c == '_' || !c.is_ascii();
    if first == '-' {
        match chars.clone().next() {
            Some(c) if ident_start(c) || c == '-' => {}
            _ => return false,
        }
    } else if !ident_start(first) {
        return false;
    }
    chars.all(|c| ident_start(c) || c.is_ascii_digit() || c == '-')
}

/// Raw start/end placements for one grid item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GridPlacements {
    pub row_start: Option<GridLine>,
    pub column_start: Option<GridLine>,
    pub row_end: Option<GridLine>,
    pub column_end: Option<GridLine>,
}

/// Parse `grid-column` / `grid-row`.
pub fn parse_grid_axis_shorthand(value: &str) -> Option<(GridLine, GridLine)> {
    let parts = split_top_level(value, '/')?;
    let start = GridLine::parse(parts.first()?)?;
    let end = match parts.get(1) {
        Some(v) => GridLine::parse(v)?,
        // CSS fills an omitted end line with the start line when that is a
        // custom-ident, and with `auto` otherwise.
        None => match &start {
            GridLine::Name(_) => start.clone(),
            _ => GridLine::Auto,
        },
    };
    Some((start, end))
}

/// Parse the four-value `grid-area` shorthand.
pub fn parse_grid_area(value: &str) -> Option<GridPlacements> {
    let parts = split_top_level(value, '/')?;
    if parts.len() > 4 {
        return None;
    }
    let mut lines: Vec<GridLine> = Vec::with_capacity(4);
    for part in &parts {
        lines.push(GridLine::parse(part)?);
    }
    while lines.len() < 4 {
        lines.push(GridLine::Auto);
    }
    // CSS expands missing values: an end edge copies its start edge when the
    // start is a custom-ident (so `grid-area: head` addresses the whole area)
    // and is `auto` otherwise.
    let same = |line: &GridLine| -> Option<GridLine> {
        matches!(line, GridLine::Name(_)).then(|| line.clone())
    };
    let row_start = lines[0].clone();
    let column_start = match parts.len() {
        1 => same(&row_start).unwrap_or(GridLine::Auto),
        _ => lines[1].clone(),
    };
    let row_end = match parts.len() {
        1 | 2 => same(&row_start).unwrap_or(GridLine::Auto),
        _ => lines[2].clone(),
    };
    let column_end = match parts.len() {
        1..=3 => same(&column_start).unwrap_or(GridLine::Auto),
        _ => lines[3].clone(),
    };
    Some(GridPlacements {
        row_start: Some(row_start),
        column_start: Some(column_start),
        row_end: Some(row_end),
        column_end: Some(column_end),
    })
}

/// Axis and hole-filling mode of automatic grid placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridAutoFlow {
    /// `true` for `row` (also the default), `false` for `column`.
    pub row: bool,
    /// `dense`: later auto items may back-fill earlier holes.
    pub dense: bool,
}

/// Parse `grid-auto-flow`.
pub fn parse_grid_auto_flow(value: &str) -> Option<GridAutoFlow> {
    let mut row = true;
    let mut dense = false;
    let mut saw_axis = false;
    for token in value.split_whitespace() {
        match token.to_ascii_lowercase().as_str() {
            "row" if !saw_axis => {
                row = true;
                saw_axis = true;
            }
            "column" if !saw_axis => {
                row = false;
                saw_axis = true;
            }
            "dense" if !dense => dense = true,
            _ => return None,
        }
    }
    Some(GridAutoFlow { row, dense })
}

/// Split at a top-level delimiter, ignoring delimiters inside parentheses.
fn split_top_level(value: &str, delimiter: char) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut seen_delimiter = false;
    for c in value.chars() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth = depth.checked_sub(1)?;
        }
        if c == delimiter && depth == 0 {
            parts.push(current.trim().to_string());
            current.clear();
            seen_delimiter = true;
        } else {
            current.push(c);
        }
    }
    parts.push(current.trim().to_string());
    (seen_delimiter || parts.len() > 1 || !parts[0].is_empty()).then_some(parts)
}

/// Split at top-level whitespace, preserving whitespace inside `repeat(...)`.
fn split_top_level_whitespace(value: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in value.chars() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth = depth.checked_sub(1)?;
        }
        if c.is_whitespace() && depth == 0 {
            if !current.trim().is_empty() {
                parts.push(current.trim().to_string());
                current.clear();
            }
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    (!parts.is_empty()).then_some(parts)
}

/// Expand a single `repeat(N, a b)` token. `auto-fill` / `auto-fit` become
/// one [`GridTrack::AutoRepeat`] for the layout to resolve; integer counts
/// expand eagerly. Auto-repeats cannot nest.
fn parse_repeat(token: &str) -> Option<Vec<GridTrack>> {
    let lower = token.to_ascii_lowercase();
    if !lower.starts_with("repeat(") || !token.ends_with(')') {
        return None;
    }
    let inner = &token[7..token.len() - 1];
    let mut parts = split_top_level(inner, ',')?;
    if parts.len() != 2 {
        return None;
    }
    let second = parts.pop()?;
    let count_raw = parts.pop()?;
    let count_raw = count_raw.trim();
    let auto_fit = match () {
        _ if count_raw.eq_ignore_ascii_case("auto-fill") => false,
        _ if count_raw.eq_ignore_ascii_case("auto-fit") => true,
        _ => {
            let count = count_raw.parse::<usize>().ok()?;
            if count == 0 {
                return None;
            }
            let tracks = parse_grid_track_list(&second)?;
            if tracks.iter().any(is_auto_repeat) {
                return None;
            }
            let mut out = Vec::with_capacity(tracks.len().saturating_mul(count));
            for _ in 0..count {
                out.extend(tracks.iter().cloned());
            }
            return Some(out);
        }
    };
    let tracks = parse_grid_track_list(&second)?;
    if tracks.is_empty() || tracks.iter().any(is_auto_repeat) {
        return None;
    }
    Some(vec![GridTrack::AutoRepeat {
        fit: auto_fit,
        tracks,
    }])
}

/// Whether this track is an unresolved `repeat(auto-fill | auto-fit, …)`.
fn is_auto_repeat(track: &GridTrack) -> bool {
    matches!(track, GridTrack::AutoRepeat { .. })
}

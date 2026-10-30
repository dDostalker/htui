use std::sync::Arc;

use super::length::Length;

// ============================================================
// CSS Grid values
//
// A deliberately small, typed subset of CSS Grid.  Track lists are
// parsed eagerly (including `repeat()`), while line placements remain
// raw line numbers/spans until the grid algorithm knows the final
// track count (needed for negative line numbers).  A bare custom-ident
// names a `grid-template-areas` area and resolves to its edge line at
// layout time; `grid-template-areas` itself is validated into a
// rectangular cell matrix right here.
// ============================================================

/// One track size in `grid-template-columns` / `grid-template-rows`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrack {
    /// `auto`: size to the track's content.
    Auto,
    /// A resolved CSS length or percentage.
    Length(Length),
    /// `<flex>`: share the remaining space after fixed and auto tracks.
    Fr(f32),
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
        Length::parse(s).map(GridTrack::Length)
    }

    /// Whether this track participates in flex (fr) distribution.
    pub fn is_fr(self) -> bool {
        matches!(self, GridTrack::Fr(_))
    }
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
/// The terminal grid model does not yet accept named lines, named areas,
/// `minmax()` or fit-content functions; returning `None` makes the CSS
/// parser drop the declaration instead of silently approximating it.
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
/// brackets and named-area strings are intentionally rejected.
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
        if let Some(rest) = s.strip_prefix("span") {
            let n = rest.trim().parse::<usize>().ok()?;
            return (n > 0).then_some(GridLine::Span(n));
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

/// Expand a single `repeat(N, a b)` token.
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
    let count = count_raw.trim().parse::<usize>().ok()?;
    if count == 0 {
        return None;
    }
    let tracks = parse_grid_track_list(&second)?;
    Some(tracks.repeat(count))
}

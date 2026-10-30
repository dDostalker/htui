use super::length::Length;

// ============================================================
// CSS Grid values
//
// A deliberately small, typed subset of CSS Grid.  Track lists are
// parsed eagerly (including `repeat()`), while line placements remain
// raw line numbers/spans until the grid algorithm knows the final
// track count (needed for negative line numbers).
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

/// A raw `<grid-line>` before the final track count is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridLine {
    Auto,
    /// CSS line number (1-based positive, negative counted from the end).
    Line(isize),
    Span(usize),
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
        // A bare custom-ident names a line; those names are not supported.
        let n = s.parse::<isize>().ok()?;
        (n != 0).then_some(GridLine::Line(n))
    }
}

/// Raw start/end placements for one grid item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
        None => GridLine::Auto,
    };
    Some((start, end))
}

/// Parse the four-value `grid-area` shorthand.
pub fn parse_grid_area(value: &str) -> Option<GridPlacements> {
    let parts = split_top_level(value, '/')?;
    if parts.len() > 4 {
        return None;
    }
    let mut lines = [GridLine::Auto; 4];
    for (i, part) in parts.iter().enumerate() {
        lines[i] = GridLine::parse(part)?;
    }
    // CSS expands missing trailing values: with 3 values, the fourth equals
    // the second when that second is a span; otherwise it is 1/auto. Our
    // span representation already resolves this safely.
    Some(GridPlacements {
        row_start: Some(lines[0]),
        column_start: Some(lines[1]),
        row_end: Some(lines[2]),
        column_end: Some(lines[3]),
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

use std::str::FromStr;

use crossterm::style::Color;
use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, Delimiter, ParseError, Parser as CssParser,
    ParserInput, ParserState, QualifiedRuleParser, StyleSheetParser, Token,
    parse_important,
};
use indexmap::IndexMap;
use log::{debug, warn};

use crate::tdom::node::{Node, NodeArena, NodeId, NodeType};

// ============================================================
// Box-model edge types — re-exported for downstream callers.
// Defined in `crate::html::box_edges`; `CssAttr` hands back resolved
// [`EdgeSizes`] / [`BorderSides`] values so the layout layer can consume
// them without re-parsing the CSS source.
// ============================================================

pub use crate::html::box_edges::{Border, BorderSides, BorderStyle, BoxEdges, EdgeSizes};

// CSS 选择器类型 & 匹配引擎

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Selector {
    Tag(NodeType),
    Class(String),
    Id(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

pub type CompoundSelector = Vec<Selector>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CssSelector {
    pub compounds: Vec<CompoundSelector>,
    pub combinators: Vec<Combinator>,
}

impl FromStr for CssSelector {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(anyhow::anyhow!("empty selector"));
        }
        // Split the selector chain into segments and the combinator that
        // introduces each segment. Whitespace is a descendant combinator; `>`,
        // `+`, `~` are explicit combinators. Empty segments (e.g. from `>` at
        // the start) are dropped — they're a parse error handled by the empty
        // check at the bottom.
        let (segments, combinators) = split_by_combinators(s);
        if segments.is_empty() {
            return Err(anyhow::anyhow!("empty selector"));
        }
        let mut compounds = Vec::with_capacity(segments.len());
        for seg in &segments {
            // A segment is one compound selector whose parts all apply to the
            // same element. Whitespace within a segment is impossible because
            // `split_by_combinators` already consumed it as a divider, but we
            // keep the inner split for safety against future callers.
            let mut compound = CompoundSelector::new();
            for part in seg.split_whitespace().filter(|p| !p.is_empty()) {
                compound.extend(parse_compound(part)?);
            }
            compounds.push(compound);
        }
        Ok(CssSelector {
            compounds,
            combinators,
        })
    }
}

/// Split a selector chain into segments and the combinator that introduces each
/// segment after the first.
///
/// `div p` → `(["div", "p"], [Descendant])`
/// `div > p` → `(["div", "p"], [Child])`
/// `div.foo` → `(["div.foo"], [])`
/// `div > p + span` → `(["div", "p", "span"], [Child, NextSibling])`
fn split_by_combinators(s: &str) -> (Vec<&str>, Vec<Combinator>) {
    let mut segments: Vec<&str> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut iter = s.char_indices().peekable();
    let mut start = 0;

    while let Some((i, ch)) = iter.next() {
        if !ch.is_whitespace() && !matches!(ch, '>' | '+' | '~') {
            continue;
        }
        let before = s[start..i].trim();
        if !before.is_empty() {
            segments.push(before);
        }
        // Skip whitespace surrounding the next segment, then look for an
        // explicit combinator char. An explicit `>`/`+`/`~` wins over a bare
        // whitespace (Descendant) combinator — so `div > p` is a single
        // `Child` combinator, not `Descendant + Child`.
        while let Some(&(_, next_ch)) = iter.peek() {
            if next_ch.is_whitespace() {
                iter.next();
            } else {
                break;
            }
        }
        let mut combinator = Combinator::Descendant;
        if let Some(&(_, next_ch)) = iter.peek() {
            combinator = match next_ch {
                '>' => {
                    iter.next();
                    Combinator::Child
                }
                '+' => {
                    iter.next();
                    Combinator::NextSibling
                }
                '~' => {
                    iter.next();
                    Combinator::SubsequentSibling
                }
                _ => Combinator::Descendant,
            };
            while let Some(&(_, next_ch)) = iter.peek() {
                if next_ch.is_whitespace() {
                    iter.next();
                } else {
                    break;
                }
            }
        }
        if iter.peek().is_some() {
            combinators.push(combinator);
        }
        start = iter.peek().map(|&(i, _)| i).unwrap_or(s.len());
    }

    let last = s[start..].trim();
    if !last.is_empty() {
        segments.push(last);
    }

    (segments, combinators)
}

fn parse_compound(s: &str) -> std::result::Result<CompoundSelector, anyhow::Error> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow::anyhow!("empty compound"));
    }
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if name.is_empty() {
                    return Err(anyhow::anyhow!("invalid class"));
                }
                out.push(Selector::Class(name));
            }
            '#' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if name.is_empty() {
                    return Err(anyhow::anyhow!("invalid id"));
                }
                out.push(Selector::Id(name));
            }
            _ => {
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' {
                    i += 1;
                }
                let tag: String = chars[start..i].iter().collect();
                if tag.is_empty() {
                    return Err(anyhow::anyhow!("invalid tag"));
                }
                out.push(Selector::Tag(tag_str_to_node_type(&tag)?));
            }
        }
    }
    Ok(out)
}

fn tag_str_to_node_type(tag: &str) -> std::result::Result<NodeType, anyhow::Error> {
    let t = tag.to_ascii_lowercase();
    match t.as_str() {
        "a" => Ok(NodeType::A),
        "article" => Ok(NodeType::Article),
        "aside" => Ok(NodeType::Aside),
        "button" => Ok(NodeType::Button),
        "details" => Ok(NodeType::Details),
        "div" => Ok(NodeType::Div),
        "figcaption" => Ok(NodeType::Figcaption),
        "figure" => Ok(NodeType::Figure),
        "footer" => Ok(NodeType::Footer),
        "form" => Ok(NodeType::Form),
        "header" => Ok(NodeType::Header),
        "h1" => Ok(NodeType::H1),
        "h2" => Ok(NodeType::H2),
        "h3" => Ok(NodeType::H3),
        "h4" => Ok(NodeType::H4),
        "h5" => Ok(NodeType::H5),
        "option" => Ok(NodeType::HtmlOption),
        "img" => Ok(NodeType::Img),
        "input" => Ok(NodeType::Input),
        "label" => Ok(NodeType::Label),
        "li" => Ok(NodeType::Li),
        "link" => Ok(NodeType::Link),
        "main" => Ok(NodeType::Main),
        "nav" => Ok(NodeType::Nav),
        "ol" => Ok(NodeType::Ol),
        "p" => Ok(NodeType::P),
        "section" => Ok(NodeType::Section),
        "select" => Ok(NodeType::Select),
        "span" => Ok(NodeType::Span),
        "style" => Ok(NodeType::Style),
        "summary" => Ok(NodeType::Summary),
        "textarea" => Ok(NodeType::Textarea),
        "ul" => Ok(NodeType::Ul),
        _ => Err(anyhow::anyhow!("unsupported CSS tag: {:?}", tag)),
    }
}

// ============================================================
// 匹配引擎 — 使用 arena + NodeId
// ============================================================

fn matches_css(node: &Node, sel: &Selector) -> bool {
    match sel {
        Selector::Tag(nt) => node.node_type == *nt,
        Selector::Class(name) => node.attributes.has_class(name),
        Selector::Id(id) => node
            .attributes
            .get_id()
            .map(|s| s.as_str() == id.as_str())
            .unwrap_or(false),
    }
}

fn matches_compound(node: &Node, compound: &CompoundSelector) -> bool {
    compound.iter().all(|s| matches_css(node, s))
}

/// Check whether a node matches a CSS selector chain.
#[doc(hidden)]
pub fn selector_chain_matches(arena: &NodeArena, node_id: NodeId, selector: &CssSelector) -> bool {
    let node = &arena[node_id];
    let n = selector.compounds.len();
    if n == 0 {
        return false;
    }
    if !matches_compound(node, &selector.compounds[n - 1]) {
        return false;
    }
    if n == 1 {
        return true;
    }
    match_combinators(
        arena,
        node_id,
        &selector.compounds,
        &selector.combinators,
        n - 1,
    )
}

fn match_combinators(
    arena: &NodeArena,
    node_id: NodeId,
    compounds: &[CompoundSelector],
    combinators: &[Combinator],
    idx: usize,
) -> bool {
    if idx == 0 {
        return true;
    }
    let combinator = combinators[idx - 1];
    let target = &compounds[idx - 1];
    let node = &arena[node_id];

    match combinator {
        Combinator::Descendant => {
            let mut cur = node.parent;
            while let Some(pid) = cur {
                let ancestor = &arena[pid];
                if matches_compound(ancestor, target)
                    && match_combinators(arena, pid, compounds, combinators, idx - 1)
                {
                    return true;
                }
                cur = ancestor.parent;
            }
            false
        }
        Combinator::Child => match node.parent {
            Some(pid) if matches_compound(&arena[pid], target) => {
                match_combinators(arena, pid, compounds, combinators, idx - 1)
            }
            _ => false,
        },
        Combinator::NextSibling => match prev_sibling_id(arena, node_id) {
            Some(sid) if matches_compound(&arena[sid], target) => {
                match_combinators(arena, sid, compounds, combinators, idx - 1)
            }
            _ => false,
        },
        Combinator::SubsequentSibling => {
            let mut cur = prev_sibling_id(arena, node_id);
            while let Some(sid) = cur {
                if matches_compound(&arena[sid], target)
                    && match_combinators(arena, sid, compounds, combinators, idx - 1)
                {
                    return true;
                }
                cur = prev_sibling_id(arena, sid);
            }
            false
        }
    }
}

fn prev_sibling_id(arena: &NodeArena, node_id: NodeId) -> Option<NodeId> {
    let node = &arena[node_id];
    let parent_id = node.parent?;
    let parent = &arena[parent_id];
    let mut prev = None;
    for &cid in &parent.children {
        if cid == node_id {
            return prev;
        }
        prev = Some(cid);
    }
    None
}

fn apply_css_rule(arena: &mut NodeArena, node_id: NodeId, selector: &CssSelector, css: &CssAttr) {
    if selector_chain_matches(arena, node_id, selector) {
        arena[node_id].css_attr.add(css.clone());
    }
    let children: Vec<NodeId> = arena[node_id].children.clone();
    for child_id in children {
        apply_css_rule(arena, child_id, selector, css);
    }
}

// CssAttr / CssRule

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
    /// - `2em` / `2rem` → cells (1em == 1 column; terminal has no font-size)
    ///
    /// Floats (`1.5em`, `33.33%`) are rounded to the nearest integer cell —
    /// terminals cannot render half columns. See [`Length::parse`] for the
    /// underlying value type.
    pub fn get_width(&self, term_columns: usize) -> Option<usize> {
        let raw = self.0.get("width")?;
        match Length::parse(raw)? {
            Length::Auto => None,
            len => Some(len.resolve(term_columns)),
        }
    }

    /// Resolve all four padding edges from `padding` / `padding-{side}`.
    ///
    /// Per-side longhands (`padding-top`, `padding-right`, `padding-bottom`,
    /// `padding-left`) win over the `padding` shorthand. Shorthand expansion
    /// follows the standard CSS 1-/2-/3-/4-value pattern; tokens we can't
    /// parse as a [`Length`] are treated as `0`.
    pub fn get_padding(&self, containing_width: usize) -> EdgeSizes {
        parse_edge_sizes(&self.0, "padding", containing_width)
    }

    /// Resolve all four margin edges from `margin` / `margin-{side}`.
    ///
    /// `margin: auto` is currently treated as `0` (the terminal has no
    /// centering primitive for margins). Per-side longhands win over the
    /// shorthand.
    pub fn get_margin(&self, containing_width: usize) -> EdgeSizes {
        parse_edge_sizes(&self.0, "margin", containing_width)
    }

    /// Resolve all four border sides from `border` / `border-{side}` /
    /// `border-{prop}`.
    ///
    /// Priority order:
    /// 1. Per-side shorthand (`border-top: 1px solid red`)
    /// 2. Universal properties (`border-width`, `border-style`, `border-color`)
    /// 3. Universal shorthand (`border: 1px solid red`)
    pub fn get_border(&self, containing_width: usize) -> BorderSides {
        // Step 1: collect universal values (lowest priority).
        let universal_width =
            parse_border_width(self.0.get("border-width").map(|s| s.as_str()), containing_width);
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
        );
        let right = parse_border_side(
            self.0.get("border-right"),
            self.0.get("border"),
            universal,
            containing_width,
        );
        let bottom = parse_border_side(
            self.0.get("border-bottom"),
            self.0.get("border"),
            universal,
            containing_width,
        );
        let left = parse_border_side(
            self.0.get("border-left"),
            self.0.get("border"),
            universal,
            containing_width,
        );

        BorderSides {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Resolve the full set of box-model edges in one call.
    ///
    /// Equivalent to calling [`get_padding`](Self::get_padding),
    /// [`get_margin`](Self::get_margin) and [`get_border`](Self::get_border)
    /// separately. Layout functions use this so they only need to look up
    /// each node's `css_attr` once.
    pub fn get_box_edges(&self, containing_width: usize) -> BoxEdges {
        BoxEdges {
            padding: self.get_padding(containing_width),
            border: self.get_border(containing_width),
            margin: self.get_margin(containing_width),
        }
    }
}

/// 解析 CSS 颜色字符串 → crossterm::Color
///
/// 支持格式：
/// - 命名颜色 (含 dark 变体): red, darkblue, DarkBlue, ...
/// - 十六进制: #RRGGBB, #RGB
/// - rgb(): rgb(255, 0, 0)
/// - 回退到 crossterm 内置 FromStr
/// Parse a CSS color string into a [`crossterm::style::Color`].
#[doc(hidden)]
pub fn parse_color_str(s: &str) -> Option<Color> {
    let lower = s.trim().to_lowercase();

    // 1. 十六进制 #RRGGBB 或 #RGB
    if let Some(hex) = lower.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    // 2. rgb(r, g, b)
    if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        return parse_rgb_color(inner);
    }

    // 3. 扩展命名颜色（含 dark 变体）
    if let Some(color) = parse_named_color(&lower) {
        return Some(color);
    }

    // 4. 回退到 crossterm 内置解析
    Color::from_str(s).ok()
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.trim();
    let (r, g, b) = if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        (r, g, b)
    } else if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
        let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
        let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
        (r, g, b)
    } else {
        return None;
    };
    Some(Color::Rgb { r, g, b })
}

fn parse_rgb_color(inner: &str) -> Option<Color> {
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].trim().parse::<u8>().ok()?;
    let g = parts[1].trim().parse::<u8>().ok()?;
    let b = parts[2].trim().parse::<u8>().ok()?;
    Some(Color::Rgb { r, g, b })
}

fn parse_named_color(name: &str) -> Option<Color> {
    match name {
        "black" => Some(Color::Black),
        "darkgrey" | "darkgray" | "grey" | "gray" => Some(Color::Grey),
        "red" => Some(Color::Red),
        "darkred" => Some(Color::DarkRed),
        "green" => Some(Color::Green),
        "darkgreen" => Some(Color::DarkGreen),
        "yellow" => Some(Color::Yellow),
        "darkyellow" => Some(Color::DarkYellow),
        "blue" => Some(Color::Blue),
        "darkblue" => Some(Color::DarkBlue),
        "magenta" => Some(Color::Magenta),
        "darkmagenta" => Some(Color::DarkMagenta),
        "cyan" => Some(Color::Cyan),
        "darkcyan" => Some(Color::DarkCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

// ============================================================
// Box-model shorthand parsing
// ============================================================

/// Expand `padding` / `padding-{side}` (or `margin` / `margin-{side}`) into an
/// [`EdgeSizes`].
///
/// Each value is parsed by [`Length::parse`]; unparseable tokens become `0`.
/// Per-side longhands are layered on top of the shorthand so `padding: 0;
/// padding-top: 5px;` resolves to top=5, others=0.
fn parse_edge_sizes(
    decls: &IndexMap<String, String>,
    shorthand: &str,
    containing_width: usize,
) -> EdgeSizes {
    let shorthand_raw = decls.get(shorthand);
    let mut edges = parse_shorthand_lengths(shorthand_raw.map(|s| s.as_str()), containing_width);

    // Layer per-side longhands on top (they take precedence).
    let top_raw = decls.get(&format!("{shorthand}-top")).map(|s| s.as_str());
    if let Some(v) = parse_length_token(top_raw, containing_width) {
        edges.top = v;
    }
    let right_raw = decls
        .get(&format!("{shorthand}-right"))
        .map(|s| s.as_str());
    if let Some(v) = parse_length_token(right_raw, containing_width) {
        edges.right = v;
    }
    let bottom_raw = decls
        .get(&format!("{shorthand}-bottom"))
        .map(|s| s.as_str());
    if let Some(v) = parse_length_token(bottom_raw, containing_width) {
        edges.bottom = v;
    }
    let left_raw = decls.get(&format!("{shorthand}-left")).map(|s| s.as_str());
    if let Some(v) = parse_length_token(left_raw, containing_width) {
        edges.left = v;
    }

    edges
}

/// Expand a 1-/2-/3-/4-value CSS shorthand into [`EdgeSizes`].
///
/// Tokenisation is whitespace-driven — we split on runs of whitespace and try
/// to parse each chunk as a [`Length`]. Unparseable tokens are skipped (treated
/// as `0`) so a stray `auto` or junk doesn't poison the whole rule.
fn parse_shorthand_lengths(value: Option<&str>, containing_width: usize) -> EdgeSizes {
    let Some(value) = value else {
        return EdgeSizes::default();
    };
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<usize> = tokens
        .iter()
        .filter_map(|t| parse_length_token(Some(t), containing_width))
        .collect();
    match parsed.len() {
        0 => EdgeSizes::default(),
        1 => EdgeSizes::new(parsed[0], parsed[0], parsed[0], parsed[0]),
        2 => EdgeSizes::new(parsed[0], parsed[1], parsed[0], parsed[1]),
        3 => EdgeSizes::new(parsed[0], parsed[1], parsed[2], parsed[1]),
        _ => EdgeSizes::new(parsed[0], parsed[1], parsed[2], parsed[3]),
    }
}

/// Parse one CSS length token to an absolute cell count.
///
/// `auto` / missing / unsupported unit → `None` (caller decides what default
/// to apply).
fn parse_length_token(value: Option<&str>, containing_width: usize) -> Option<usize> {
    let raw = value?;
    match Length::parse(raw)? {
        Length::Auto => None,
        len => Some(len.resolve(containing_width)),
    }
}

/// Parse a `border-width` value to cells. Mirrors [`parse_length_token`]; any
/// value CSS accepts as a length works (including `thin` / `medium` / `thick`
/// once added to [`Length::parse`]).
fn parse_border_width(value: Option<&str>, containing_width: usize) -> usize {
    parse_length_token(value, containing_width).unwrap_or(0)
}

/// Map a `border-style` keyword to a [`BorderStyle`]. Unknown / missing
/// defaults to [`BorderStyle::None`].
fn parse_border_style(value: Option<&str>) -> BorderStyle {
    let Some(value) = value else {
        return BorderStyle::None;
    };
    BorderStyle::parse(value)
}

/// Build one [`Border`] edge from `border-{side}` (priority), falling back to
/// the universal `border` shorthand, finally to the resolved
/// `border-width`/`-style`/`-color` defaults.
///
/// The `fallback` argument already encodes the universal defaults resolved by
/// the caller — passing it down means we don't re-parse the same map entries
/// four times.
fn parse_border_side(
    side_raw: Option<&String>,
    shorthand_raw: Option<&String>,
    fallback: Border,
    containing_width: usize,
) -> Border {
    if let Some(value) = side_raw {
        if let Some(parsed) = parse_border_shorthand(value, containing_width) {
            return parsed;
        }
        // `border-top` was present but unparseable — fall through to next
        // priority rather than dropping the side entirely.
    }
    if let Some(value) = shorthand_raw {
        if let Some(parsed) = parse_border_shorthand(value, containing_width) {
            return parsed;
        }
    }
    fallback
}

/// Parse a `border` shorthand declaration.
///
/// CSS allows any of width / style / color in any order, with some pieces
/// optional. We tokenise on whitespace and classify each token; tokens we
/// can't classify fall through (treated as missing) so a typo in one piece
/// doesn't drop the whole border.
fn parse_border_shorthand(value: &str, containing_width: usize) -> Option<Border> {
    let mut width: Option<usize> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<Color> = None;

    for token in value.split_whitespace() {
        if let Some(w) = parse_length_token(Some(token), containing_width) {
            width = Some(w);
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "solid" | "dashed" | "dotted" | "double" | "none" | "hidden" | "groove" | "ridge"
                | "inset" | "outset"
        ) {
            style = Some(BorderStyle::parse(token));
            continue;
        }
        if let Some(c) = parse_color_str(token) {
            color = Some(c);
            continue;
        }
        // Unrecognised token — ignore (don't fail the whole rule).
    }

    // At least width or style must be present; otherwise the value wasn't a
    // border shorthand (e.g. someone wrote `border: inherit;`).
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }

    Some(Border {
        width: width.unwrap_or(0),
        style: style.unwrap_or(BorderStyle::None),
        color,
    })
}

/// A typed CSS length value resolved against the terminal's monospace grid.
///
/// Terminals render in **cells**, not pixels or points. Every length we
/// accept is normalised to an integer column count by [`Length::resolve`].
///
/// Accepted forms:
/// - `auto`                         → [`Length::Auto`]   (caller decides what to fill)
/// - `40px`, bare `40`              → [`Length::Cells(n)`]
/// - `50%`                          → [`Length::Pct(p)`] (raw 0–100, divided at resolve)
/// - `2em`, `1.5em`                 → [`Length::Em(n)`]
/// - `2rem`, `1.5rem`               → [`Length::Rem(n)`]
///
/// Rejected (returns `None` from [`Length::parse`]):
/// - `vw`, `vh`, `vmin`, `vmax` — the terminal has no viewport. Use `%`.
/// - `pt`, `cm`, `mm`, `in`, `pc` — no DPI. Use `px` (= cells).
/// - Negative or signed (`+`, `-`) values — CSS forbids these for `width`.
/// - Trailing junk after the value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// `auto` — defer to the containing block.
    Auto,
    /// Absolute cell count (CSS `40px` or bare `40`).
    Cells(usize),
    /// Percentage 0..=100 of the containing block (CSS `50%`).
    Pct(f32),
    /// Em multiplier (CSS `2em`). 1em == 1 cell on the terminal grid.
    Em(f32),
    /// Root-em multiplier (CSS `2rem`). Same semantics as `em` here — the
    /// terminal has no root font-size distinction.
    Rem(f32),
}

/// Owned mirror of the cssparser [`Token`] variants we accept. Lets
/// [`Length::parse`] release its borrow on the parser before checking for
/// trailing junk with `is_exhausted()`.
#[derive(Debug, Clone, Copy)]
enum PreLength {
    Cells(f32),
    Pct(f32),
    Em(f32),
    Rem(f32),
}

impl Length {
    /// Parse a CSS length token into a typed [`Length`].
    ///
    /// Returns `None` for unsupported units (`vw`/`vh`/`pt`/`cm`/...), signed
    /// values, trailing garbage, or empty input. The literal `auto` (any case)
    /// always returns [`Length::Auto`].
    pub fn parse(s: &str) -> Option<Length> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("auto") {
            return Some(Length::Auto);
        }
        // cssparser's tokenizer handles `40`, `40.5`, `40px`, `1.5em`, `50%`,
        // escapes, and unicode digits uniformly. We pin the lifetime with a
        // named ParserInput binding — see the warning in cssparser's docs:
        // a temporary `ParserInput::new(...)` would be dropped before the
        // parser finished reading.
        let mut input_buf = ParserInput::new(s);
        let mut input = CssParser::new(&mut input_buf);
        // Convert the borrowed `&Token` into an owned `PreLength` so we can
        // later call `input.is_exhausted()` to check for trailing junk.
        // Keeping the `&Token` alive would borrow `input` mutably and conflict
        // with that second call.
        let pre = match input.next().ok()? {
            Token::Number {
                value,
                int_value,
                has_sign: false,
                ..
            } => PreLength::Cells(int_value.map(|i| i as f32).unwrap_or(*value)),
            Token::Dimension {
                value,
                int_value,
                has_sign: false,
                unit,
                ..
            } => {
                let n = int_value.map(|i| i as f32).unwrap_or(*value);
                match unit.to_ascii_lowercase().as_str() {
                    "px" => PreLength::Cells(n),
                    "em" => PreLength::Em(n),
                    "rem" => PreLength::Rem(n),
                    // vw / vh / vmin / vmax / pt / cm / mm / in / pc / ...
                    _ => return None,
                }
            }
            Token::Percentage {
                unit_value,
                has_sign: false,
                ..
            } => {
                // cssparser divides by 100 for us (`50%` → `0.5`). Multiply back
                // so the stored value matches what the user wrote.
                PreLength::Pct(unit_value * 100.0)
            }
            _ => return None,
        };
        // Reject any non-whitespace junk after the value token.
        if !input.is_exhausted() {
            return None;
        }
        Some(match pre {
            PreLength::Cells(n) => Length::Cells(n.round() as usize),
            PreLength::Pct(p) => Length::Pct(p),
            PreLength::Em(n) => Length::Em(n),
            PreLength::Rem(n) => Length::Rem(n),
        })
    }

    /// Resolve the length into a concrete column count given the containing
    /// block's width in cells.
    ///
    /// - `Auto`                       → `containing` (fill the container)
    /// - `Cells(n)`                   → `n`
    /// - `Pct(p)`                     → round(containing * p / 100), min 1
    /// - `Em(n)` / `Rem(n)`           → round(n), clamped at 0
    ///
    /// Floats round **away from zero** (Rust's `f32::round`), so `0.5em` is
    /// 1 cell and `2.5em` is 3 cells — never zero, never half.
    pub fn resolve(&self, containing: usize) -> usize {
        match self {
            Length::Auto => containing,
            &Length::Cells(n) => n,
            &Length::Pct(p) => {
                let raw = (containing as f32 * p / 100.0).round();
                (raw as usize).max(1)
            }
            &Length::Em(n) => n.round().max(0.0) as usize,
            &Length::Rem(n) => n.round().max(0.0) as usize,
        }
    }
}

/// Text alignment direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Font weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontWeight {
    Normal,
    Bold,
}

/// Font style (CSS `font-style`).
///
/// Mirrors the CSS values. Terminal output collapses `Italic` and `Oblique`
/// to the same renderer flag, but the distinction is preserved here so the
/// raw CSS decision is not lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyleKind {
    Normal,
    Italic,
    Oblique,
}

/// Text decoration (CSS `text-decoration`).
///
/// The CSS property is space-separated and can combine multiple values; we
/// model the subset we render (`underline`, `line-through`) as discrete
/// variants so `==` matches expected outputs without set semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecoration {
    None,
    Underline,
    LineThrough,
    UnderlineLineThrough,
}

#[derive(Debug, Clone)]
pub struct CssRule(pub IndexMap<CssSelector, CssAttr>);

impl CssRule {
    pub fn new() -> Self {
        CssRule(IndexMap::new())
    }
    pub fn add(&mut self, sel: CssSelector, attr: IndexMap<String, String>) {
        self.0.insert(sel, CssAttr(attr));
    }
}

// Parser

#[derive(Debug, Clone)]
pub struct Parser {
    context: String,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        Parser {
            context: input.to_string(),
        }
    }

    pub fn parse(&mut self) -> std::result::Result<CssRule, anyhow::Error> {
        let mut parser_input = ParserInput::new(&self.context);
        let mut parser = CssParser::new(&mut parser_input);
        let mut state = CssParseState::new();
        {
            let rule_list = StyleSheetParser::new(&mut parser, &mut state);
            for rule_result in rule_list {
                if let std::result::Result::Err(e) = rule_result {
                    warn!("CSS error: {:?}", e);
                }
            }
        }
        let mut css = CssRule::new();
        for (sel_text, declarations) in state.rules.drain(..) {
            match CssSelector::from_str(&sel_text) {
                std::result::Result::Ok(sel) => css.add(sel, declarations),
                std::result::Result::Err(e) => warn!("Selector parse fail: {}: {}", sel_text, e),
            }
        }
        debug!("CSS parsed: {} rules", css.0.len());
        Ok(css)
    }
}

struct CssParseState {
    rules: Vec<(String, IndexMap<String, String>)>,
}
impl CssParseState {
    fn new() -> Self {
        Self { rules: vec![] }
    }
}

impl<'i> QualifiedRuleParser<'i> for CssParseState {
    type Prelude = String;
    type QualifiedRule = ();
    type Error = cssparser::BasicParseError<'i>;
    fn parse_prelude<'t>(
        &mut self,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<String, ParseError<'i, Self::Error>> {
        let start = input.position();
        while !input.is_exhausted() {
            if input.next_including_whitespace_and_comments().is_err() {
                break;
            }
        }
        let text = input.slice_from(start).trim().to_string();
        if text.is_empty() {
            return Err(input.new_error(BasicParseErrorKind::QualifiedRuleInvalid));
        }
        Ok(text)
    }
    fn parse_block<'t>(
        &mut self,
        prelude: String,
        _start: &ParserState,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<(), ParseError<'i, Self::Error>> {
        let mut declarations = IndexMap::new();
        parse_declarations_into(input, &mut declarations);
        self.rules.push((prelude, declarations));
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for CssParseState {
    type Prelude = ();
    type AtRule = ();
    type Error = cssparser::BasicParseError<'i>;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<(), ParseError<'i, Self::Error>> {
        debug!("skip @{}", name);
        while !input.is_exhausted() {
            input.next().ok();
        }
        Ok(())
    }
    fn parse_block<'t>(
        &mut self,
        _: (),
        _: &ParserState,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<(), ParseError<'i, Self::Error>> {
        while !input.is_exhausted() {
            input.next().ok();
        }
        Ok(())
    }
    fn rule_without_block(&mut self, _: (), _: &ParserState) -> std::result::Result<(), ()> {
        Ok(())
    }
}

/// Trim a declaration value and strip any `!important` token that survived
/// token-level parsing (defensive — the parser usually consumes it).
fn clean_value_text(raw: &str) -> String {
    let trimmed = raw.trim();
    match trimmed.find("!important") {
        Some(idx) => trimmed[..idx].trim_end().to_string(),
        None => trimmed.to_string(),
    }
}

/// Parse a CSS declaration block text into key-value pairs.
///
/// Built on top of [`cssparser`]'s tokenizer via [`parse_declarations_into`]
/// so it correctly handles:
/// - `/* ... */` block comments (including those inside string literals)
/// - the `!important` marker
/// - missing trailing `;`
/// - escaped identifiers and unicode in property names
/// - quoted string values with semicolons / colons inside
#[doc(hidden)]
pub fn parse_declaration_text(block: &str) -> IndexMap<String, String> {
    let mut input_buf = ParserInput::new(block);
    let mut input = CssParser::new(&mut input_buf);
    let mut map = IndexMap::new();
    parse_declarations_into(&mut input, &mut map);
    map
}

/// Walk every `name: value [!important]` declaration in `input` and store
/// them in `out`. Tolerates malformed input by skipping to the next `;`
/// rather than aborting the whole block.
fn parse_declarations_into<'i, 't>(
    input: &mut CssParser<'i, 't>,
    out: &mut IndexMap<String, String>,
) {
    loop {
        input.skip_whitespace();
        if input.is_exhausted() {
            return;
        }
        let name = match input.expect_ident() {
            Ok(s) => s.clone(),
            Err(_) => {
                // Malformed declaration (e.g. starts with `;` or unknown
                // token). Skip ahead to the next semicolon to avoid an
                // infinite loop on the same junk token.
                let _: Result<(), ParseError<()>> = input.parse_until_after(
                    Delimiter::Semicolon,
                    |_| Ok(()),
                );
                continue;
            }
        };
        if input.expect_colon().is_err() {
            // `name` without `:` — advance past the bogus declaration.
            let _: Result<(), ParseError<()>> = input.parse_until_after(
                Delimiter::Semicolon,
                |_| Ok(()),
            );
            continue;
        }
        let value_start = input.position();
        // Consume value tokens until `;` or `!important` is reached.
        let _: Result<(), ParseError<()>> = input.parse_until_before(
            Delimiter::Semicolon | Delimiter::Bang,
            |_| Ok(()),
        );
        let value_text = input.slice_from(value_start);
        let _important = input.try_parse(parse_important).is_ok();
        let cleaned = clean_value_text(value_text);
        if !name.is_empty() && !cleaned.is_empty() {
            out.insert(name.to_string(), cleaned);
        }
        // Consume the trailing `;` if present.
        let _ = input.try_parse(|i| i.expect_semicolon().map(|_| ()));
    }
}

// css_into_node

pub fn css_into_node(
    arena: &mut NodeArena,
    root_id: NodeId,
    css: &CssRule,
) -> std::result::Result<(), anyhow::Error> {
    for (selector, attr) in css.0.iter() {
        apply_css_rule(arena, root_id, selector, attr);
    }
    // Propagate inheritable CSS properties (color, text-align, font-weight,
    // font-style, text-decoration)
    inherit_css(arena, root_id, &IndexMap::new());
    Ok(())
}

/// All CSS properties the renderer recognises by name.
///
/// Single source of truth for property names. Adding support for a new property:
///   1. Append the name here.
///   2. Add an accessor on [`CssAttr`] (e.g. `get_padding()`).
///   3. If it should inherit, also append to [`INHERITED_PROPS`].
///
/// No other site needs editing — the cascade in [`inherit_css`] drives itself
/// from these lists.
pub const SUPPORTED_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "background",
    "text-align",
    "font-weight",
    "font-style",
    "text-decoration",
    "width",
    // Box-model edges
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-width",
    "border-style",
    "border-color",
];

/// Inheritable subset of [`SUPPORTED_PROPERTIES`]: a child without an explicit
/// value picks up its parent's.
///
/// Kept as a separate list because Rust const-fn filtering isn't yet stable;
/// appending a name here is what flips a property to inheritable.
pub const INHERITED_PROPS: &[&str] = &[
    "color",
    "text-align",
    "font-weight",
    "font-style",
    "text-decoration",
];

fn inherit_css(arena: &mut NodeArena, node_id: NodeId, parent_attrs: &IndexMap<String, String>) {
    // Merge parent inheritable attrs into current node (if not already set)
    let node = &mut arena[node_id];
    for key in INHERITED_PROPS {
        if let Some(value) = parent_attrs.get(*key) {
            node.css_attr
                .0
                .entry((*key).to_string())
                .or_insert_with(|| value.clone());
        }
    }

    // Collect inheritable attrs from this node to pass down
    let mut merged = parent_attrs.clone();
    for key in INHERITED_PROPS {
        if let Some(value) = node.css_attr.0.get(*key) {
            merged.insert((*key).to_string(), value.clone());
        }
    }

    // Recurse into children (clone to avoid borrow issues)
    let children: Vec<NodeId> = node.children.clone();
    for child_id in children {
        inherit_css(arena, child_id, &merged);
    }
}

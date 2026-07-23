use std::str::FromStr;

use crossterm::style::Color;
use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, ParseError, Parser as CssParser, ParserInput,
    ParserState, QualifiedRuleParser, StyleSheetParser,
};
use indexmap::IndexMap;
use log::{debug, warn};

use crate::tdom::node::{Node, NodeArena, NodeId, NodeType};

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
        let segments = split_by_combinators(s);
        let mut compounds = Vec::new();
        let mut combinators = Vec::new();
        for (i, seg) in segments.iter().enumerate() {
            let sub: Vec<&str> = seg.split_whitespace().filter(|p| !p.is_empty()).collect();
            for (j, part) in sub.iter().enumerate() {
                compounds.push(parse_compound(part)?);
                if j < sub.len() - 1 {
                    combinators.push(Combinator::Descendant);
                }
            }
            if i < segments.len() - 1 {
                combinators.push(extract_combinator(s));
            }
        }
        if compounds.is_empty() {
            return Err(anyhow::anyhow!("empty selector"));
        }
        Ok(CssSelector {
            compounds,
            combinators,
        })
    }
}

fn split_by_combinators(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        if ch == '>' || ch == '+' || ch == '~' {
            let before = s[start..i].trim();
            if !before.is_empty() {
                result.push(&s[start..i]);
            } else {
                result.push("");
            }
            start = i + ch.len_utf8();
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() || result.is_empty() {
        result.push(&s[start..]);
    }
    result
}

fn extract_combinator(s: &str) -> Combinator {
    for ch in s.chars() {
        match ch {
            '>' => return Combinator::Child,
            '+' => return Combinator::NextSibling,
            '~' => return Combinator::SubsequentSibling,
            _ => {}
        }
    }
    Combinator::Descendant
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
        "htmloption" => Ok(NodeType::HtmlOption),
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
    /// Element width (CSS `width`).
    ///
    /// - `80%` → 80% of terminal width
    /// - `40px` → fixed 40 columns
    /// - `auto` / missing → `None` (fill available)
    pub fn get_width(&self, term_columns: usize) -> Option<usize> {
        let raw = self.0.get("width")?;
        let s = raw.trim();
        if s == "auto" {
            return None;
        }
        if let Some(pct) = s.strip_suffix('%') {
            let p: f32 = pct.trim().parse().ok()?;
            return Some(((term_columns as f32 * p / 100.0) as usize).max(1));
        }
        if let Some(n) = s.strip_suffix("px") {
            return n.trim().parse::<usize>().ok();
        }
        // 其他单位或无单位 → 占满
        None
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
        let block_start = input.position();
        while !input.is_exhausted() {
            input.next().ok();
        }
        let block_text = input.slice_from(block_start).to_string();
        self.rules
            .push((prelude, parse_declaration_text(&block_text)));
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

/// Parse a CSS declaration block text into key-value pairs.
#[doc(hidden)]
pub fn parse_declaration_text(block: &str) -> IndexMap<String, String> {
    let mut m = IndexMap::new();
    let mut k = String::new();
    let mut v = String::new();
    let mut in_v = false;
    for c in block.chars() {
        if !in_v {
            if c == ':' {
                in_v = true;
            } else {
                k.push(c);
            }
        } else if c == ';' {
            let kt = k.trim().to_string();
            if !kt.is_empty() {
                m.insert(kt, v.trim().to_string());
            }
            k.clear();
            v.clear();
            in_v = false;
        } else {
            v.push(c);
        }
    }
    let kt = k.trim().to_string();
    if !kt.is_empty() {
        m.insert(kt, v.trim().to_string());
    }
    m
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
    // Propagate inheritable CSS properties (color, text-align, font-weight)
    inherit_css(arena, root_id, &IndexMap::new());
    Ok(())
}

/// CSS properties that propagate from parent to child when the child has no explicit value.
///
/// Modelled as a typed enum so the set cannot drift from the accessors on [`CssAttr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum InheritableProperty {
    Color,
    TextAlign,
    FontWeight,
}

impl InheritableProperty {
    /// Storage key used inside [`CssAttr`]'s `IndexMap`.
    fn css_name(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::TextAlign => "text-align",
            Self::FontWeight => "font-weight",
        }
    }
}

/// Inheritable CSS properties: propagate from parent to child if child has no own value.
const INHERITABLE: &[InheritableProperty] = &[
    InheritableProperty::Color,
    InheritableProperty::TextAlign,
    InheritableProperty::FontWeight,
];

fn inherit_css(arena: &mut NodeArena, node_id: NodeId, parent_attrs: &IndexMap<String, String>) {
    // Merge parent inheritable attrs into current node (if not already set)
    let node = &mut arena[node_id];
    for prop in INHERITABLE {
        let key = prop.css_name();
        if let Some(value) = parent_attrs.get(key) {
            node.css_attr
                .0
                .entry(key.to_string())
                .or_insert_with(|| value.clone());
        }
    }

    // Collect inheritable attrs from this node to pass down
    let mut merged = parent_attrs.clone();
    for prop in INHERITABLE {
        let key = prop.css_name();
        if let Some(value) = node.css_attr.0.get(key) {
            merged.insert(key.to_string(), value.clone());
        }
    }

    // Recurse into children (clone to avoid borrow issues)
    let children: Vec<NodeId> = node.children.clone();
    for child_id in children {
        inherit_css(arena, child_id, &merged);
    }
}

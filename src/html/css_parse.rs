use std::{collections::HashMap, str::FromStr};

use crossterm::style::Color;
use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, ParseError, Parser as CssParser, ParserInput,
    ParserState, QualifiedRuleParser, StyleSheetParser,
};
use log::{debug, trace, warn};

use crate::tdom::node::{Node, NodeType};

// ============================================================
// CSS 选择器类型 & 匹配引擎
// ============================================================

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
                combinators.push(extract_combinator(s, segments[i]));
            }
        }

        if compounds.is_empty() {
            return Err(anyhow::anyhow!("empty selector after parsing"));
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

fn extract_combinator(s: &str, _after: &str) -> Combinator {
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
        return Err(anyhow::anyhow!("empty compound selector"));
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
                    return Err(anyhow::anyhow!("invalid class in compound {:?}", s));
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
                    return Err(anyhow::anyhow!("invalid id in compound {:?}", s));
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
                    return Err(anyhow::anyhow!("invalid tag in compound {:?}", s));
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
        "main" => Ok(NodeType::Main),
        "nav" => Ok(NodeType::Nav),
        "ol" => Ok(NodeType::Ol),
        "p" => Ok(NodeType::P),
        "section" => Ok(NodeType::Section),
        "select" => Ok(NodeType::Select),
        "span" => Ok(NodeType::Span),
        "summary" => Ok(NodeType::Summary),
        "textarea" => Ok(NodeType::Textarea),
        "ul" => Ok(NodeType::Ul),
        _ => Err(anyhow::anyhow!("unsupported tag in CSS selector: {:?}", tag)),
    }
}

// ============================================================
// 匹配引擎 — 利用 parent 指针支持完整组合子
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

fn selector_chain_matches(node: &Node, selector: &CssSelector) -> bool {
    let compounds = &selector.compounds;
    let combinators = &selector.combinators;
    let n = compounds.len();
    if n == 0 {
        return false;
    }
    if !matches_compound(node, &compounds[n - 1]) {
        return false;
    }
    if n == 1 {
        return true;
    }
    match_combinators(node, compounds, combinators, n - 1)
}

fn match_combinators(
    node: &Node,
    compounds: &[CompoundSelector],
    combinators: &[Combinator],
    idx: usize,
) -> bool {
    if idx == 0 {
        return true;
    }
    let combinator = combinators[idx - 1];
    let target = &compounds[idx - 1];
    match combinator {
        Combinator::Descendant => {
            let mut current = node.parent_ref();
            while let Some(ancestor) = current {
                if matches_compound(ancestor, target)
                    && match_combinators(ancestor, compounds, combinators, idx - 1)
                {
                    return true;
                }
                current = ancestor.parent_ref();
            }
            false
        }
        Combinator::Child => match node.parent_ref() {
            Some(parent) if matches_compound(parent, target) => {
                match_combinators(parent, compounds, combinators, idx - 1)
            }
            _ => false,
        },
        Combinator::NextSibling => match prev_sibling(node) {
            Some(prev) if matches_compound(prev, target) => {
                match_combinators(prev, compounds, combinators, idx - 1)
            }
            _ => false,
        },
        Combinator::SubsequentSibling => {
            let mut sibling = prev_sibling(node);
            while let Some(prev) = sibling {
                if matches_compound(prev, target)
                    && match_combinators(prev, compounds, combinators, idx - 1)
                {
                    return true;
                }
                sibling = prev_sibling(prev);
            }
            false
        }
    }
}

fn prev_sibling(node: &Node) -> Option<&Node> {
    let parent = node.parent_ref()?;
    let self_ptr = node as *const Node;
    let mut prev = None;
    for child in &parent.children {
        if std::ptr::eq(child, self_ptr) {
            return prev;
        }
        prev = Some(child);
    }
    None
}

fn apply_css_rule(node: &mut Node, selector: &CssSelector, css: &CssAttr) {
    if selector_chain_matches(node, selector) {
        node.css_attr.add(css.clone());
    }
    for child in &mut node.children {
        apply_css_rule(child, selector, css);
    }
}

// ============================================================
// CssAttr / CssRule
// ============================================================

#[derive(Debug, Clone, Default)]
pub struct CssAttr(HashMap<String, String>);

impl CssAttr {
    pub fn new() -> Self {
        CssAttr(HashMap::new())
    }
    fn add(&mut self, css: CssAttr) {
        self.0.extend(css.0);
    }
    pub fn get_color(&self) -> Option<Color> {
        let color = self.0.get("color")?;
        Color::from_str(color).ok()
    }
}

#[derive(Debug, Clone)]
pub struct CssRule(pub(crate) HashMap<CssSelector, CssAttr>);

impl CssRule {
    pub fn new() -> Self {
        CssRule(HashMap::new())
    }
    fn add(&mut self, sele: CssSelector, attr: HashMap<String, String>) {
        self.0.insert(sele, CssAttr(attr));
    }
    pub fn css_to_node(&self) {
        todo!()
    }
}

// ============================================================
// Parser
// ============================================================

#[derive(Debug, Clone)]
pub struct Parser {
    context: String,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        debug!("创建 CSS Parser，输入长度 {} 字节", input.len());
        Parser {
            context: input.to_string(),
        }
    }

    pub fn parse(&mut self) -> std::result::Result<CssRule, anyhow::Error> {
        debug!("开始解析 CSS");

        let mut parser_input = ParserInput::new(&self.context);
        let mut parser = CssParser::new(&mut parser_input);
        let mut state = CssParseState::new();

        {
            let rule_list = StyleSheetParser::new(&mut parser, &mut state);
            for rule_result in rule_list {
                match rule_result {
                    std::result::Result::Ok(_) => {}
                    std::result::Result::Err(e) => {
                        warn!("CSS 规则解析错误: {:?}", e);
                    }
                }
            }
        }

        let mut css = CssRule::new();
        for (sel_text, declarations) in state.rules.drain(..) {
            match CssSelector::from_str(&sel_text) {
                std::result::Result::Ok(selector) => {
                    css.add(selector, declarations);
                }
                std::result::Result::Err(e) => {
                    warn!("选择器解析失败 '{}': {}", sel_text, e);
                }
            }
        }

        debug!("CSS 解析完成，共 {} 条规则", css.0.len());
        Ok(css)
    }
}

struct CssParseState {
    rules: Vec<(String, HashMap<String, String>)>,
}

impl CssParseState {
    fn new() -> Self {
        Self { rules: Vec::new() }
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
        trace!("CSS 选择器文本: '{}'", text);
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
            let _ = input.next();
        }
        let block_text = input.slice_from(block_start).to_string();
        let declarations = parse_declaration_text(&block_text);

        debug!("CSS 规则: '{}' → {} 条声明", prelude, declarations.len());
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
        debug!("跳过 CSS at-rule: @{}", name);
        while !input.is_exhausted() {
            let _ = input.next();
        }
        Ok(())
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: (),
        _start: &ParserState,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<(), ParseError<'i, Self::Error>> {
        while !input.is_exhausted() {
            let _ = input.next();
        }
        Ok(())
    }

    fn rule_without_block(
        &mut self,
        _prelude: (),
        _start: &ParserState,
    ) -> std::result::Result<(), ()> {
        Ok(())
    }
}

fn parse_declaration_text(block: &str) -> HashMap<String, String> {
    let mut declarations = HashMap::new();
    let mut key = String::new();
    let mut value = String::new();
    let mut in_value = false;

    for c in block.chars() {
        if !in_value {
            if c == ':' {
                in_value = true;
            } else {
                key.push(c);
            }
        } else if c == ';' {
            let k = key.trim().to_string();
            if !k.is_empty() {
                declarations.insert(k, value.trim().to_string());
            }
            key.clear();
            value.clear();
            in_value = false;
        } else {
            value.push(c);
        }
    }

    let k = key.trim().to_string();
    if !k.is_empty() {
        declarations.insert(k, value.trim().to_string());
    }

    declarations
}

// ============================================================
// css_into_node
// ============================================================

pub fn css_into_node(
    node: &mut Node,
    css: &CssRule,
) -> std::result::Result<(), anyhow::Error> {
    for (selector, attr) in css.0.iter() {
        apply_css_rule(node, selector, attr);
    }
    Ok(())
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn build_node_with_parent() -> Node {
        let mut root = Node::new();
        let mut div = Node::from(NodeType::Div);
        div.attributes =
            crate::tdom::attributes::Attr::parse_from_str(r#"class="active" id="main""#);
        let mut p = Node::from(NodeType::P);
        p.children.push(Node::text("Hello"));
        div.children.push(p);
        root.children.push(div);
        root.fix_parent_pointers();
        root
    }

    #[test]
    fn test_parse_simple_rule() {
        init();
        let mut parser = Parser::new("p { color: red; }");
        let css = parser.parse().unwrap();
        assert_eq!(css.0.len(), 1);
    }

    #[test]
    fn test_parse_descendant_selector() {
        init();
        let sel: CssSelector = "div p".parse().unwrap();
        assert_eq!(sel.combinators.len(), 1);
        assert_eq!(sel.combinators[0], Combinator::Descendant);
    }

    #[test]
    fn test_parse_child_selector() {
        init();
        let sel: CssSelector = "div > p".parse().unwrap();
        assert_eq!(sel.combinators[0], Combinator::Child);
    }

    #[test]
    fn test_parse_next_sibling() {
        init();
        let sel: CssSelector = "h1 + p".parse().unwrap();
        assert_eq!(sel.combinators[0], Combinator::NextSibling);
    }

    #[test]
    fn test_matches_descendant() {
        init();
        let root = build_node_with_parent();
        let selector: CssSelector = "div p".parse().unwrap();
        let p = &root.children[0].children[0];
        assert!(selector_chain_matches(p, &selector));
    }

    #[test]
    fn test_matches_child() {
        init();
        let root = build_node_with_parent();
        let selector: CssSelector = "div > p".parse().unwrap();
        let p = &root.children[0].children[0];
        assert!(selector_chain_matches(p, &selector));
    }

    #[test]
    fn test_css_into_node_applies_color() {
        init();
        let mut root = build_node_with_parent();
        let mut css = CssRule::new();
        let sel: CssSelector = "div > p".parse().unwrap();
        let mut attrs = HashMap::new();
        attrs.insert("color".into(), "red".into());
        css.add(sel, attrs);
        css_into_node(&mut root, &css).unwrap();
        let p = &root.children[0].children[0];
        assert!(p.css_attr.get_color().is_some());
    }

    #[test]
    fn test_css_comments() {
        init();
        let mut parser = Parser::new("/* comment */ p { color: red; }");
        let css = parser.parse().unwrap();
        assert_eq!(css.0.len(), 1);
    }

    #[test]
    fn test_full_css_pipeline_like_main() {
        init();
        let css_str = "h1 { color: cyan; } h2 { color: yellow; } p { color: grey; } .highlight { color: magenta; }";
        let html_str = "<div><h1>Title</h1><p>Text</p><p class=\"highlight\">Highlighted</p></div>";

        let mut css_parser = Parser::new(css_str);
        let css = css_parser.parse().unwrap();
        assert!(!css.0.is_empty());

        let mut html_parser = crate::html::html_parse::Parser::new(html_str);
        let mut node = html_parser.parse().unwrap();
        node.fix_parent_pointers();
        css_into_node(&mut node, &css).unwrap();

        fn find_by_type<'a>(node: &'a Node, nt: &NodeType) -> Vec<&'a Node> {
            let mut result = Vec::new();
            if node.node_type == *nt {
                result.push(node);
            }
            for child in &node.children {
                result.extend(find_by_type(child, nt));
            }
            result
        }

        let h1s = find_by_type(&node, &NodeType::H1);
        assert!(!h1s.is_empty());
        assert!(h1s[0].css_attr.get_color().is_some());

        let ps = find_by_type(&node, &NodeType::P);
        let highlighted = ps.iter().find(|p| p.attributes.has_class("highlight"));
        assert!(highlighted.is_some());
        assert!(highlighted.unwrap().css_attr.get_color().is_some());
    }

    #[test]
    fn test_class_selector_match() {
        init();
        let sel: CssSelector = ".red".parse().unwrap();
        let mut p = Node::from(NodeType::P);
        p.attributes = crate::tdom::attributes::Attr::parse_from_str(r#"class="red blue""#);
        assert!(selector_chain_matches(&p, &sel));
    }

    #[test]
    fn test_multiple_class_rules() {
        init();
        let css_str = ".red { color: red; } .blue { color: blue; }";
        let html_str = "<div><p class=\"red\">R</p><p class=\"blue\">B</p></div>";

        let mut css_parser = Parser::new(css_str);
        let css = css_parser.parse().unwrap();
        assert_eq!(css.0.len(), 2);

        let mut html_parser = crate::html::html_parse::Parser::new(html_str);
        let mut node = html_parser.parse().unwrap();
        node.fix_parent_pointers();
        css_into_node(&mut node, &css).unwrap();

        fn find_p_by_class<'a>(node: &'a Node, class: &str) -> Option<&'a Node> {
            if node.node_type == NodeType::P && node.attributes.has_class(class) {
                return Some(node);
            }
            for child in &node.children {
                if let Some(found) = find_p_by_class(child, class) {
                    return Some(found);
                }
            }
            None
        }

        let red_p = find_p_by_class(&node, "red").expect("找不到 .red P");
        let blue_p = find_p_by_class(&node, "blue").expect("找不到 .blue P");
        assert!(red_p.css_attr.get_color().is_some());
        assert!(blue_p.css_attr.get_color().is_some());
    }

    #[test]
    fn test_full_render_pipeline_without_terminal() {
        init();
        let html = "<div><h1>Title</h1><p>Hello <em>world</em>!</p></div>";
        let css = "h1 { color: cyan; } p { color: grey; }";

        let mut html_parser = crate::html::html_parse::Parser::new(html);
        let mut css_parser = Parser::new(css);
        let css_rules = css_parser.parse().unwrap();
        let mut node = html_parser.parse().unwrap();
        node.fix_parent_pointers();
        css_into_node(&mut node, &css_rules).unwrap();

        assert!(!node.children.is_empty());
        let div = &node.children[0];
        assert_eq!(div.node_type, NodeType::Div);
        assert!(!div.children.is_empty());

        let h1 = &div.children[0];
        assert!(h1.css_attr.get_color().is_some());

        // Layout rendering with fixed terminal size
        use crate::{layout::layout::TermMap, tui::graph_api::TermLayout};
        let term_map = TermMap { columns: 80, rows: 24 };
        let mut term_layout = TermLayout::new(&term_map).unwrap();
        term_layout.draw(&node, &term_map).unwrap();

        let non_empty: Vec<_> = term_layout
            .layout
            .iter()
            .filter(|c| c.as_str() != " ")
            .collect();
        assert!(!non_empty.is_empty(), "画布上应有绘制内容");

        let display_str = format!("{}", term_layout);
        assert!(!display_str.is_empty(), "Display 输出不应为空");
    }
}

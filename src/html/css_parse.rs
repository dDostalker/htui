use std::{collections::HashMap, str::FromStr};

use anyhow::{Ok, Result, anyhow};
use crossterm::style::Color;

use crate::tdom::node::{Node, NodeType};
/// the Parsers State of the css attribute
/// when read ':' , the state turn Value
/// when read ';' , the state turn Key
enum State {
    Key,
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Selector {
    Tag(NodeType),
    Class(String),
    Id(String),
}

pub type CompoundSelector = Vec<Selector>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CssSelector {
    pub compounds: Vec<CompoundSelector>,
}

impl FromStr for CssSelector {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(anyhow!("empty selector"));
        }
        let parts: Vec<&str> = s.split_whitespace().filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Err(anyhow!("empty selector"));
        }
        let mut compounds = Vec::with_capacity(parts.len());
        for p in parts {
            compounds.push(parse_compound(p)?);
        }
        Ok(CssSelector { compounds })
    }
}

/// from str的方法，主要作用用于
/// 将 `div`、`div.foo`、`#id`、`.c` 等解析为组合选择器
///
fn parse_compound(s: &str) -> Result<CompoundSelector> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty compound selector"));
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
                    return Err(anyhow!("invalid class in compound {:?}", s));
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
                    return Err(anyhow!("invalid id in compound {:?}", s));
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
                    return Err(anyhow!("invalid tag in compound {:?}", s));
                }
                out.push(Selector::Tag(tag_str_to_node_type(&tag)?));
            }
        }
    }
    Ok(out)
}

fn tag_str_to_node_type(tag: &str) -> Result<NodeType> {
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
        _ => Err(anyhow!("unsupported tag in CSS selector: {:?}", tag)),
    }
}

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

/// 在 ancestors中按顺序匹配compounds
fn matches_subsequence_chain(ancestors: &[&Node], compounds: &[CompoundSelector]) -> bool {
    if compounds.is_empty() {
        return true;
    }
    fn dfs(
        ancestors: &[&Node],
        compounds: &[CompoundSelector],
        anc_start: usize,
        step: usize,
    ) -> bool {
        if step == compounds.len() {
            return true;
        }
        for i in anc_start..ancestors.len() {
            if matches_compound(ancestors[i], &compounds[step])
                && dfs(ancestors, compounds, i + 1, step + 1)
            {
                return true;
            }
        }
        false
    }
    dfs(ancestors, compounds, 0, 0)
}

fn ancestors_ptr_to_refs(ancestors: &[*const Node]) -> Vec<&Node> {
    ancestors.iter().map(|p| unsafe { &**p }).collect()
}

/// 当前节点是否匹配整条选择器链
fn selector_chain_matches(node: &Node, ancestors: &[*const Node], selector: &CssSelector) -> bool {
    let compounds = &selector.compounds;
    if compounds.is_empty() {
        return false;
    }
    let last = compounds.len() - 1;
    if !matches_compound(node, &compounds[last]) {
        return false;
    }
    if last == 0 {
        return true;
    }
    let arefs = ancestors_ptr_to_refs(ancestors);
    matches_subsequence_chain(&arefs, &compounds[0..last])
}

fn apply_css_rule(
    node: &mut Node,
    ancestors: &[*const Node],
    selector: &CssSelector,
    css: &CssAttr,
) {
    if selector_chain_matches(node, ancestors, selector) {
        node.css_attr.add(css.clone());
    }
    let mut path: Vec<*const Node> = ancestors.to_vec();
    path.push(node as *const Node);
    for child in node.children.iter_mut() {
        apply_css_rule(child, &path, selector, css);
    }
}

/// CssAttr newtype to store css attributes
/// ```text
/// aaa : bbb;
/// ccc : ddd;
/// ```
/// to
/// ```text
/// CssAttr {
///     aaa: "bbb",
///     ccc: "ddd",
/// }
/// ```
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
/// CssRule newtype to store css rules
/// ```text
/// aaa {
///     bbb : ccc;
///     ddd : eee;
/// }
/// bbb {
///     bbb : ccc;
///     ddd : eee;
/// }
/// ```
/// to
/// ```text
/// CssRule {
///     aaa: CssAttr {
///         bbb: "ccc",
///         ddd: "eee",
///     },
///     bbb: CssAttr {
///         bbb: "ccc",
///         ddd: "eee",
///     },
/// }
/// ```
#[derive(Debug, Clone)]
pub struct CssRule(HashMap<CssSelector, CssAttr>);

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

#[derive(Debug, Clone)]
pub struct Parser {
    point: usize,
    context: String,
    line: usize,
    column: usize,
}

impl Parser {
    fn _next_char(&mut self) -> Result<Option<char>> {
        if self._eof() {
            return Ok(None);
        }
        match self.context[self.point..].chars().next() {
            Some(c) => {
                self.point += c.len_utf8();
                self.column += 1;
                Ok(Some(c))
            }
            None => Err(anyhow!(format!("bad char in parser:{:?}", self))),
        }
    }
    /// like peek
    fn _next_char_prev(&self) -> Result<Option<char>> {
        if self._eof() {
            return Ok(None);
        }
        match self.context[self.point..].chars().next() {
            Some(c) => Ok(Some(c)),
            None => Err(anyhow!(format!("bad char in parser:{:?}", self))),
        }
    }

    fn _eof(&self) -> bool {
        self.point >= self.context.len()
    }
}
impl Parser {
    pub fn new(input: &str) -> Self {
        Parser {
            context: input.to_string(),
            point: 0,
            line: 1,
            column: 1,
        }
    }
    pub fn parse(&mut self) -> Result<CssRule> {
        let mut css = CssRule::new();
        while let Some((sele, attrs)) = self._parse()? {
            css.add(sele, attrs);
        }
        Ok(css)
    }

    fn parse_from_block(input: &str) -> Result<CssAttr> {
        let mut parser = Parser::new(input);
        let attrs = parser.get_css_attr(input.to_string())?;
        Ok(CssAttr(attrs))
    }

    fn _parse(&mut self) -> Result<Option<(CssSelector, HashMap<String, String>)>> {
        let sele = self.get_selector()?;

        if let Some(sele) = sele {
            let attr_block = self.get_attr_block()?;
            let attrs = self.get_css_attr(attr_block)?;
            return Ok(Some((sele, attrs)));
        }
        Ok(None)
    }
    /// 从css parse内获取当前块的css的选择器 并返回选择器类型
    fn get_selector(&mut self) -> Result<Option<CssSelector>> {
        let start = self.point;
        loop {
            let c = self._next_char()?;
            if c.is_none() {
                if self._eof() {
                    return Ok(None);
                }
                return Err(anyhow!("cannot find selector end"));
            }
            let c = c.unwrap();
            if c == '{' {
                if start + 1 == self.point {
                    return Err(anyhow!("cannot find selector"));
                } else {
                    break;
                }
            }
        }
        self.point -= 1;
        CssSelector::from_str(self.context[start..self.point].trim()).map(Some)
    }

    /// 获取当前块的css属性块 并返回属性块内容
    fn get_attr_block(&mut self) -> Result<String> {
        let start = self.point;

        loop {
            let c = self._next_char()?;
            if c.is_none() {
                return Err(anyhow!("cannot find attribute block end"));
            }
            let c = c.unwrap();
            if c == '}' {
                break;
            }
        }

        Ok(self.context[start + 1..self.point - 1].to_string())
    }
    /// 解析css块中的内容，用于解析后返回，这里关联变量 State 用于表明解析状态
    /// 这里考虑后续是不是需要把String去重写？
    fn get_css_attr(&mut self, content: String) -> Result<HashMap<String, String>> {
        let mut attrs = HashMap::new();
        let mut key = String::new();
        let mut value = String::new();
        let mut state = State::Key;

        for c in content.chars() {
            match state {
                State::Key => {
                    if c == ':' {
                        state = State::Value;
                    } else {
                        key.push(c);
                    }
                }
                State::Value => {
                    if c == ';' {
                        attrs.insert(key.trim().to_string(), value.trim().to_string());
                        key.clear();
                        value.clear();
                        state = State::Key;
                    } else {
                        value.push(c);
                    }
                }
            }
        }

        let k = key.trim().to_string();
        let v = value.trim().to_string();
        if !k.is_empty() {
            attrs.insert(k, v);
        }
        Ok(attrs)
    }
}
/// 用于将css中解析的内容写入node节点
pub fn css_into_node(node: &mut Node, css: &CssRule) -> Result<()> {
    for (selector, attr) in css.0.iter() {
        apply_css_rule(node, &[] as &[*const Node], selector, attr);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tdom::node::NodeType;

    #[test]
    fn test_get_selector() {
        let mut parser = Parser::new("p { color: red;}");
        assert_eq!(
            parser.get_selector().unwrap().unwrap(),
            "p".parse::<CssSelector>().unwrap()
        );
    }

    #[test]
    fn test_get_css_attr() {
        let content = "color: red;".to_string();
        let mut parser = Parser::new(""); // 解析器状态好像无所谓？
        let attrs = parser.get_css_attr(content).unwrap();
        assert_eq!(attrs.get("color"), Some(&"red".to_string()));
    }

    #[test]
    fn test_get_attr_block() {
        let mut parser = Parser::new("{color: red;}");
        assert_eq!(parser.get_attr_block().unwrap(), "color: red;");
    }

    #[test]
    fn test_parse() {
        let mut parser = Parser::new(
            "p {
            color: red;
            background-color: blue
            }",
        );
        let attrs = parser.parse().unwrap();
        dbg!(&attrs);
    }

    #[test]
    fn test_parse_from_block() {
        let attrs = Parser::parse_from_block("color: red; cos: blue;").unwrap();
        dbg!(&attrs);
    }

    #[test]
    fn test_malformed_css() {
        let css = "p { color: red; "; // 缺少
        let mut parser = Parser::new(css);
        let result = parser.parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_selector_class_id_compound() {
        let s: CssSelector = ".foo".parse().unwrap();
        assert_eq!(
            s,
            CssSelector {
                compounds: vec![vec![Selector::Class("foo".into())]],
            }
        );
        let s: CssSelector = "#bar".parse().unwrap();
        assert_eq!(
            s,
            CssSelector {
                compounds: vec![vec![Selector::Id("bar".into())]],
            }
        );
        let s: CssSelector = "div.active".parse().unwrap();
        assert_eq!(
            s,
            CssSelector {
                compounds: vec![vec![
                    Selector::Tag(NodeType::Div),
                    Selector::Class("active".into()),
                ]],
            }
        );
    }

    #[test]
    fn test_selector_descendant() {
        let s: CssSelector = "div p".parse().unwrap();
        assert_eq!(
            s.compounds,
            vec![
                vec![Selector::Tag(NodeType::Div)],
                vec![Selector::Tag(NodeType::P)],
            ]
        );
    }

    #[test]
    fn test_matches_compound_and_chain() {
        let mut div = Node::from(NodeType::Div);
        div.attributes = crate::tdom::attributes::Attr::parse_from_str(r#"class="active""#);
        assert!(matches_compound(
            &div,
            &vec![
                Selector::Tag(NodeType::Div),
                Selector::Class("active".into()),
            ]
        ));

        let p = Node::from(NodeType::P);
        let ancestors: &[*const Node] = &[&div as *const Node];
        let sel: CssSelector = "div p".parse().unwrap();
        assert!(selector_chain_matches(&p, ancestors, &sel));

        let p2 = Node::from(NodeType::P);
        let ancestors2: &[*const Node] = &[];
        assert!(!selector_chain_matches(&p2, ancestors2, &sel));
    }
}

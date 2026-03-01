use std::{collections::HashMap, str::FromStr};

use anyhow::{Ok, Result, anyhow};

use crate::tdom::node::{Node, NodeType};
/// the Parsers State of the css attribute
/// when read ':' , the state turn Value
/// when read ';' , the state turn Key
enum State {
    Key,
    Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Selector {
    Class(NodeType),
    Id(String),
    Tag(String),
}
impl FromStr for Selector {
    type Err = anyhow::Error;
    /// 优先分出selector类型
    fn from_str(select: &str) -> Result<Self, Self::Err> {
        match select {
            "." => todo!(),
            "#" => todo!(),
            _ => Ok(Self::_string_to_tag(select)),
        }
    }
}
impl Selector {
    fn _string_to_tag(select: &str) -> Self {
        match select {
            "p" => Self::Tag(select.to_string()),
            _ => todo!(),
        }
    }
}

/// CssAttr newtype to store css attributes
/// ```
/// aaa : bbb;
/// ccc : ddd;
/// ```
/// to
/// ```
/// CssAttr {
///     aaa: "bbb",
///     ccc: "ddd",
/// }
/// ```
#[derive(Debug, Clone)]
struct CssAttr(HashMap<String, String>);
/// CssRule newtype to store css rules
/// ```
/// aaa {
///     bbb : ccc;
///     ddd : eee;
/// }
/// ```
/// to
/// ```
/// CssRule {
///     aaa: CssAttr {
///         bbb: "ccc",
///         ddd: "eee",
///     },
/// }
/// ```
#[derive(Debug, Clone)]
pub struct CssRule(HashMap<Selector, CssAttr>);

impl CssRule {
    pub fn new() -> Self {
        CssRule(HashMap::new())
    }
    fn add(&mut self, sele: Selector, attr: HashMap<String, String>) {
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
        let ret = match self.context.chars().nth(self.point) {
            Some(c) => {
                self.point += c.len_utf8();
                self.column += 1;
                Ok(Some(c))
            }
            None => Err(anyhow!(format!("bad char in parser:{:?}", self))),
        };
        ret
    }
    /// like peek
    fn _next_char_prev(&self) -> Result<Option<char>> {
        if self._eof() {
            return Ok(None);
        }
        let ret = match self.context.chars().nth(self.point) {
            Some(c) => Ok(Some(c)),
            None => Err(anyhow!(format!("bad char in parser:{:?}", self))),
        };
        ret
    }

    fn _eof(&self) -> bool {
        self.point >= self.context.len()
    }
}
impl Parser {
    fn new(input: &str) -> Self {
        Parser {
            context: input.to_string(),
            point: 0,
            line: 1,
            column: 1,
        }
    }
    pub fn parse(&mut self) -> Result<CssRule> {
        let mut css = CssRule::new();
        loop {
            if let Some((sele, attrs)) = self._parse()? {
                css.add(sele, attrs);
            } else {
                break;
            };
        }
        Ok(css)
    }
    fn parse_from_block(input: &str) -> Result<CssAttr> {
        let mut parser = Parser::new(input);
        let attrs = parser.get_css_attr(input.to_string())?;
        return Ok(CssAttr(attrs));
    }

    fn _parse(&mut self) -> Result<Option<(Selector, HashMap<String, String>)>> {
        let sele = self.get_selector()?;

        if let Some(sele) = sele {
            let attr_block = self.get_attr_block()?;
            let attrs = self.get_css_attr(attr_block)?;
            return Ok(Some((sele, attrs)));
        }
        Ok(None)
    }
    /// 从css parse内获取当前块的css的选择器 并返回选择器类型
    fn get_selector(&mut self) -> Result<Option<Selector>> {
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
        let selector = Selector::from_str(self.context[start..self.point].trim())?;
        Ok(Some(selector))
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

        Ok(attrs)
    }
}
/// 用于将css中解析的内容写入node节点
fn css_into_node(node: &mut Node, css: &CssRule) -> Result<()> {
    for (selector, attr) in css.0.iter() {
        match selector {
            Selector::Class(class) => {
                // 处理类选择器
                todo!()
            }
            Selector::Id(id) => {
                // 处理ID选择器
                todo!()
            }
            Selector::Tag(tag) => {
                // 处理标签选择器
                todo!()
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_selector() {
        let mut parser = Parser::new("div");
        assert_eq!(
            parser.get_selector().unwrap().unwrap(),
            Selector::Class(NodeType::Div)
        );
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
            background-color: blue;
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
}

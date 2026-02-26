use std::collections::HashMap;

use anyhow::{Ok, Result, anyhow};
/// the Parsers State of the css attribute
/// when read ':' , the state turn Value
/// when read ';' , the state turn Key
enum State {
    Key,
    Value,
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
struct CssRule(HashMap<String, CssAttr>);

impl CssRule {
    fn new() -> Self {
        CssRule(HashMap::new())
    }
    fn add(&mut self, sele: String, attr: HashMap<String, String>) {
        self.0.insert(sele, CssAttr(attr));
    }
}

#[derive(Debug, Clone)]
struct Parser {
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
    fn new(input: &str) -> Self {
        Parser {
            context: input.to_string(),
            point: 0,
            line: 1,
            column: 1,
        }
    }
    fn parse(&mut self) -> Result<CssRule> {
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

    fn _parse(&mut self) -> Result<Option<(String, HashMap<String, String>)>> {
        let sele = self.get_selector()?;

        if let Some(sele) = sele {
            let attr_block = self.get_attr_block()?;
            let attrs = self.get_css_attr(attr_block)?;
            return Ok(Some((sele, attrs)));
        }
        Ok(None)
    }

    fn get_selector(&mut self) -> Result<Option<String>> {
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

        Ok(Some(self.context[start..self.point].trim().to_string()))
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_selector() {
        let mut parser = Parser::new("div");
        assert_eq!(parser.get_selector().unwrap().unwrap(), "div".to_string());
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
}

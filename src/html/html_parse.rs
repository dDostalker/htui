use crate::tdom::{
    attributes::Attr,
    element_fn::ELEMENT_FUNCTIONS,
    node::{Node, NodeType},
};
pub(crate) use anyhow::{Ok, Result, anyhow};
use log::debug;
use std::iter::Peekable;
use std::str::CharIndices;

/// parser , to rememeber the status of html parser
#[derive(Debug)]
pub struct Parser<'a> {
    input: &'a str,
    chars: Peekable<CharIndices<'a>>,
    line: usize,
    column: usize,
}

/// To solve the question when we have a html like :
/// ```html
/// start<p>hello</p>
/// ```
/// we should return
///
/// [Str("start"),Block("<p>"),...EndBlock("</p>")]
///
/// Str mean a string without element Block       ...
///
/// the Block mean a string have a element Block   <...>
///
/// EndBlock mean a string end with a element Block </...>
///
/// Enter mean "\n"
#[derive(Debug)]
enum EleBlockRead<'a> {
    Str(&'a str),
    Block(&'a str),
    EndBlock(&'a str),
    Enter,
}

///Parser machine
impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.char_indices().peekable(),
            line: 1,
            column: 1,
        }
    }

    /// 获取当前位置的字节索引
    fn current_byte_index(&mut self) -> usize {
        match self.chars.peek() {
            Some((idx, _)) => *idx,
            None => self.input.len(),
        }
    }

    /// 消费一个字符，更新行号和列号
    fn consume_char(&mut self) -> Option<char> {
        if let Some((_, c)) = self.chars.next() {
            if c == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
            Some(c)
        } else {
            None
        }
    }

    /// read element from parser content, when start with <,end with >
    ///
    /// example:
    /// ```html
    ///     <span>hello</span>
    /// ```
    ///[Block("<span>"),Str("hello"),BlockEnd("</span>")]
    fn _element_content(&mut self) -> Result<Option<EleBlockRead<'a>>> {
        let start = self.current_byte_index();

        // Peek 第一个字符
        let first_char = match self.chars.peek() {
            Some(&(_, c)) => c,
            None => return Ok(None),
        };

        let ret = if first_char == '<' {
            self.consume_char(); // 消耗 '<'
            loop {
                match self.consume_char() {
                    Some('>') => break,
                    Some(_) => continue,
                    None => break, // EOF before '>'
                }
            }
            let end = self.current_byte_index();
            let content = &self.input[start..end];
            if content.starts_with("</") {
                Ok(Some(EleBlockRead::EndBlock(content)))
            } else {
                Ok(Some(EleBlockRead::Block(content)))
            }
        } else {
            // 读取普通文本直到遇到 '<' 或 EOF
            loop {
                match self.chars.peek() {
                    Some(&(_, '<')) => break, // 遇到 '<' 停止，不消耗
                    Some(_) => {
                        self.consume_char();
                    } // 消耗普通字符
                    None => break,
                }
            }
            let end = self.current_byte_index();
            let mut ret = &self.input[start..end];

            // if the str only "\n", del it.
            if ret.contains('\n') {
                ret = ret.trim();
            }

            if ret.is_empty() {
                Ok(Some(EleBlockRead::Enter))
            } else {
                Ok(Some(EleBlockRead::Str(ret)))
            }
        };

        #[cfg(debug_assertions)]
        {
            debug!("element_content {:?}  parser:{:?}", ret, self);
        }
        ret
    }
}

impl<'a> Parser<'a> {
    pub fn parse(&mut self) -> Result<Node> {
        self._parse(Some(NodeType::Global))
    }
    /// todo
    fn _parse(&mut self, nodetype: Option<NodeType>) -> Result<Node> {
        let mut node = Node::new();
        loop {
            let element = self._element_content()?;
            if let Some(ele) = element {
                match ele {
                    EleBlockRead::Block(content) => {
                        let (next_node_type, attr) = Self::check_ele_ty(content)?;

                        if next_node_type.is_self_closing() {
                            let mut self_closing_node = Node::from(next_node_type);
                            self_closing_node.attributes = attr;
                            node.children.push(self_closing_node);
                        } else {
                            let mut child_node = self._parse(Some(next_node_type))?;
                            child_node.attributes = attr;
                            node.children.push(child_node);
                        }
                    }
                    EleBlockRead::Str(content) => {
                        if let Some(nty) = nodetype.clone() {
                            node.node_type = nty;
                            node.children.push(Node::text(content));
                        } else {
                            node = Node::text(content);
                        }
                    }
                    EleBlockRead::EndBlock(content) => {
                        let (end_nodetype, _attr) = Self::check_ele_ty(content)?;
                        if let Some(nodetype) = nodetype.clone()
                            && end_nodetype == nodetype
                        {
                            node.node_type = end_nodetype;
                            return Ok(node);
                        } else {
                            return Err(anyhow!(format!(
                                "bad close with {:?}..{:?} \n parser:{:?}",
                                nodetype, end_nodetype, self
                            )));
                        }
                    }
                    EleBlockRead::Enter => {
                        continue;
                    }
                }
            } else {
                break;
            }
        }

        Ok(node)
    }
    /// Simple check element type
    pub fn check_ele_ty(element: &str) -> Result<(NodeType, Attr)> {
        for func in ELEMENT_FUNCTIONS {
            match func(element)? {
                Some((nodetype, attr)) => return Ok((nodetype, attr)),
                None => continue,
            }
        }
        Err(anyhow!(format!("Unsupported element type {}", element)))
    }
}
#[cfg(test)]
mod tests {
    use crate::{html::html_parse::Parser, tdom::node::NodeType};

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }
    #[test]
    fn test_check_ele_ty() {
        init();
        assert_eq!(Parser::check_ele_ty("<span>").unwrap().0, NodeType::Span);
        assert_eq!(Parser::check_ele_ty("<div>").unwrap().0, NodeType::Div);
        assert!(Parser::check_ele_ty("<unsupported>").is_err());
    }
    #[test]
    fn test_parse_html() {
        init();
        let html = "start<span>
            <div>
                <p>Hello</p>
                <p>World</p>
            </div>
        </span>
        <span>
            <div>
                <p>Hello</p>
            </div>
        </span>";
        // Update: use Parser::new instead of struct initialization
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }
    #[test]
    fn test_parse_with_attributes() {
        init();
        let html = r#"<div class="container" id="main"><p>Hello</p></div>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }
    #[test]
    fn test_parse_img() {
        init();
        let html = r#"<div><img src="test.jpg" alt="Test Image"></div>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }
    #[test]
    fn test_parse_headings() {
        init();
        let html = r#"
            <div>
                <h1>Main Title</h1>
                <h2>Subtitle</h2>
                <h3>Section</h3>
                <h4>Subsection</h4>
                <h5>Small heading</h5>
                <h6>Smallest heading</h6>
            </div>
        "#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }

    #[test]
    fn test_heading_with_attributes() {
        init();
        let html = r#"<h1 id="title" class="main-title">Welcome</h1>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();

        // 验证解析结果
        if let Some(child) = node.children.first() {
            assert_eq!(child.node_type, NodeType::H1);
            assert_eq!(child.attributes.get("id"), Some(&"title".to_string()));
            assert_eq!(
                child.attributes.get_class(),
                Some(vec!["main-title".to_string()])
            );
        }
    }

    #[test]
    fn test_nested_headings() {
        init();
        let html = r#"
            <div>
                <h1>Chapter 1</h1>
                <p>Introduction</p>
                <h2>Section 1.1</h2>
                <p>Content</p>
                <h2>Section 1.2</h2>
                <p>More content</p>
            </div>
        "#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }
    #[test]
    fn test_single_heading() {
        init();
        let html = r#"<h1>Test</h1>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();
        dbg!(&node);
    }
}

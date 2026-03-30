use crate::tdom::{
    attributes::Attr,
    element_fn::ELEMENT_FUNCTIONS,
    node::{Node, NodeType},
};
pub(crate) use anyhow::{Result, anyhow};
use log::{debug, error, trace, warn};
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
        debug!("开始解析 HTML，输入长度 {} 字节", self.input.len());
        let root = self._parse(Some(NodeType::Global))?;
        debug!("解析完成，根节点子节点数量: {}", root.children.len());
        Ok(root)
    }
    /// todo
    fn _parse(&mut self, nodetype: Option<NodeType>) -> Result<Node> {
        let mut node = Node::new();
        loop {
            let element = match self._element_content() {
                Ok(el) => el,
                Err(e) => {
                    error!(
                        "_element_content 失败 line {}, col {}: {}",
                        self.line, self.column, e
                    );
                    return Err(e);
                }
            };
            #[cfg(debug_assertions)]
            {
                if std::env::var("HTML_PARSE_ELEMENT").unwrap_or("0".to_string()) == "1".to_string()
                {
                    println!(
                        "self._element_content()? => {:?} nodetype is {:?}",
                        element, nodetype
                    )
                }
            }
            if let Some(ele) = element {
                match ele {
                    EleBlockRead::Block(content) => {
                        trace!("发现开始标签: {}", content.trim());

                        let (next_node_type, attr) = match Self::check_ele_ty(content) {
                            Ok(res) => res,
                            Err(e) => {
                                warn!("不支持或解析失败的开始标签: {} → {}", content, e);
                                continue;
                            }
                        };

                        debug!("识别为开始标签 → {:?}，属性: {:?}", next_node_type, attr);

                        if next_node_type.is_self_closing() {
                            let mut self_closing_node = Node::from(next_node_type.clone());
                            self_closing_node.attributes = attr;
                            node.children.push(self_closing_node);
                            trace!("添加自闭合节点 {:?}", next_node_type);
                        } else {
                            debug!("递归解析子节点 {:?}", next_node_type);
                            let mut child_node = match self._parse(Some(next_node_type.clone())) {
                                Ok(n) => n,
                                Err(e) => {
                                    error!("子节点 {:?} 递归解析失败: {}", next_node_type, e);
                                    return Err(e);
                                }
                            };
                            child_node.attributes = attr;
                            node.children.push(child_node);
                        }
                    }
                    EleBlockRead::Str(content) => {
                        if let Some(nty) = nodetype.clone() {
                            trace!(
                                "发现文本内容 (长度 {}): {:?}",
                                content.len(),
                                content.trim()
                            );
                            node.node_type = nty;
                            node.children.push(Node::text(content));
                        } else {
                            node = Node::text(content);
                        }
                    }
                    EleBlockRead::EndBlock(content) => {
                        trace!("发现结束标签: {}", content.trim());

                        let (end_nodetype, _) = match Self::check_ele_ty(content) {
                            Ok(res) => res,
                            Err(e) => {
                                error!("结束标签格式错误 {}: {}", content, e);
                                return Err(e);
                            }
                        };
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
        trace!("尝试匹配元素: {}", element.trim());

        for func in ELEMENT_FUNCTIONS {
            match func(element) {
                Ok(Some((nodetype, attr))) => {
                    trace!("匹配成功 → {:?}", nodetype);
                    return Ok((nodetype, attr));
                }
                Ok(None) => continue,
                Err(e) => {
                    error!("元素函数执行出错: {}", e);
                    return Err(e);
                }
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
    fn test_heading_with_attributes() {
        init();
        let html = r#"<h1 id="title" class="main-title" src="./1/2/3">Welcome</h1>"#;
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();

        // 验证解析结果
        if let Some(child) = node.children.first() {
            assert_eq!(child.node_type, NodeType::H1);
            assert_eq!(child.attributes.get("id"), Some(&"title".to_string()));
            assert_eq!(child.attributes.get("src"), Some(&"./1/2/3".to_string()));
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

use crate::tdom::{
    attributes::Attr,
    element_fn::ELEMENT_FUNCTIONS,
    node::{Node, NodeType},
};
pub(crate) use anyhow::{Ok, Result, anyhow};
use log::debug;
/// parser , to rememeber the status of html parser
#[derive(Debug)]
struct Parser {
    point: usize,
    context: String,
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
impl Parser {
    /// use to get next char from input context
    /// Result to duel the nth error
    /// the Option to duel the end of file
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
    /// read element from parser content, when start with <,end with >
    ///
    /// example:
    /// ```html
    ///     <span>hello</span>
    /// ```
    ///[Block("<span>"),Str("hello"),BlockEnd("</span>")]
    fn _element_content(&mut self) -> Result<Option<EleBlockRead<'_>>> {
        let start = self.point;

        let chars = self._next_char()?;
        let ret = match chars {
            Some('<') => {
                while let Some(c) = self._next_char()? {
                    if c == '>' {
                        break;
                    }
                }
                let content = &self.context[start..self.point];
                if content.starts_with("</") {
                    Ok(Some(EleBlockRead::EndBlock(content)))
                } else {
                    Ok(Some(EleBlockRead::Block(content)))
                }
            }
            Some(_) => {
                while let Some(c) = self._next_char()? {
                    if c == '<' {
                        break;
                    }
                }
                self.point -= 1;
                let mut ret = &self.context[start..self.point];
                // if the str only "\n", del it.
                if ret.contains("\n") {
                    self.line += ret.matches("\n").count();
                    self.column = 1;
                    ret = ret.trim();
                }
                if ret.is_empty() {
                    Ok(Some(EleBlockRead::Enter))
                } else {
                    Ok(Some(EleBlockRead::Str(ret)))
                }
            }
            None => Ok(None),
        };
        #[cfg(debug_assertions)]
        {
            debug!("element_content {:?}  parser:{:?}", ret, self);
        }
        ret
    }
    fn _eof(&self) -> bool {
        self.point >= self.context.len()
    }
}

impl Parser {
    fn new(context: &str) -> Self {
        Self {
            context: context.to_string(),
            point: 0,
            line: 1,
            column: 1,
        }
    }
    fn parse(&mut self) -> Result<Node> {
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
        let mut p = Parser {
            point: 0,
            context: html.to_string(),
            line: 1,
            column: 1,
        };
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
}

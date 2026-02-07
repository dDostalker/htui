pub(crate) use anyhow::{Ok, Result, anyhow};

use crate::tdom::node::{Node, NodeType};
/// parser , to rememeber the status of html parser
struct Parser {
    point: usize,
    context: String,
}
/// the return of element read fn
/// cmp means the element is matched
/// length means the length of element
struct RleRet {
    cmp: bool,
    length: usize,
}
/// To solve the question when we have a html like :
/// ```html
/// 1111<p>1111</p>
/// ```
/// we should return
///
/// [Str("111"),Block("<p>"),...]
///
/// Str mean a string without element Block       ...
///
/// the Block mean a string have a element Block   <...>
enum EleBlockRead<'a> {
    Str(&'a str),
    Block(&'a str),
    EndBlock(&'a str),
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
        match self.context.chars().nth(self.point) {
            Some(c) => {
                self.point += c.len_utf8();
                Ok(Some(c))
            }
            None => Err(anyhow!(format!("bad char in point {}", self.point))),
        }
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
        match chars {
            Some('<') => {
                while let Some(c) = self._next_char()? {
                    if c == '>' {
                        break;
                    }
                }
                let ret = &self.context[start..self.point];
                if ret.starts_with("</") {
                    Ok(Some(EleBlockRead::EndBlock(ret)))
                } else {
                    Ok(Some(EleBlockRead::Block(ret)))
                }
            }
            Some(_) => {
                while let Some(c) = self._next_char()? {
                    if c == '<' {
                        break;
                    }
                }
                self.point -= 1;
                Ok(Some(EleBlockRead::Str(&self.context[start..self.point])))
            }
            None => Ok(None),
        }
    }
    fn _eof(&self) -> bool {
        self.point >= self.context.len()
    }
}

impl Parser {
    fn parse(&mut self) -> Result<Node> {
        self._parse(Some(NodeType::Global))
    }
    /// 递归进行块闭合生成dom结构体
    fn _parse(&mut self, nodetype: Option<NodeType>) -> Result<Node> {
        let mut node = Node::new();
        loop {
            let element = self._element_content()?;
            if let Some(ele) = element {
                match ele {
                    EleBlockRead::Block(content) => {
                        let next_node_type = Self::check_ele_ty(content)?;
                        node.children.push(self._parse(Some(next_node_type))?)
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
                        let end_nodetype = Self::check_ele_ty(content)?;
                        if let Some(nodetype) = nodetype.clone()
                            && end_nodetype == nodetype
                        {
                            node.node_type = end_nodetype;
                            return Ok(node);
                        } else {
                            return Err(anyhow!(format!(
                                "bad close with {:?}..{:?}",
                                nodetype, end_nodetype
                            )));
                        }
                    }
                }
            } else {
                break;
            }
        }

        Ok(node)
    }
    /// Simple check element type
    pub fn check_ele_ty(element: &str) -> Result<NodeType> {
        if element.starts_with("<span>") || element.starts_with("</span>") {
            Ok(NodeType::Span)
        } else if element.starts_with("<div>") || element.starts_with("</div>") {
            Ok(NodeType::Div)
        } else {
            Err(anyhow!(format!("Unsupported element type {}", element)))
        }
    }
}

#[test]
fn test_check_ele_ty() {
    assert_eq!(Parser::check_ele_ty("<span>").unwrap(), NodeType::Span);
    assert_eq!(Parser::check_ele_ty("<div>").unwrap(), NodeType::Div);
    assert!(Parser::check_ele_ty("<unsupported>").is_err());
}
#[test]
fn test_parse_html() {
    let html = "<span><div><div>Hello</div></div></span>";
    let mut p = Parser {
        point: 0,
        context: html.to_string(),
    };
    let node = p.parse().unwrap();
    dbg!(&node);
}

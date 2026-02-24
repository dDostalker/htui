use crate::tdom::attributes::Attr;

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum NodeType {
    None,
    Global,
    Span,
    Div,
    P,
    Img,
    A,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    Header,
    Footer,
    Nav,
    Main,
    Section,
    Article,
    Aside,
    Figure,
    Figcaption,
    Details,
    Summary,
    Text(String),
    // todo!
}

impl NodeType {
    pub fn is_self_closing(&self) -> bool {
        matches!(self, NodeType::Img)
    }
}

#[derive(Debug)]
pub struct Node {
    pub node_type: NodeType,
    pub children: Vec<Node>,
    pub attributes: Attr,
}

impl Node {
    pub fn new() -> Node {
        Node {
            node_type: NodeType::Global,
            children: vec![],
            attributes: Attr::new(),
        }
    }
    pub fn from(node_type: NodeType) -> Node {
        Node {
            node_type,
            children: vec![],
            attributes: Attr::new(),
        }
    }
    pub fn children(&self) -> impl Iterator<Item = &Node> {
        self.children.iter()
    }
    pub fn text(data: &str) -> Node {
        Node {
            node_type: NodeType::Text(data.to_string()),
            children: vec![],
            attributes: Attr::new(),
        }
    }
    pub fn div() -> Node {
        Node {
            node_type: NodeType::Div,
            children: vec![],
            attributes: Attr::new(),
        }
    }
}

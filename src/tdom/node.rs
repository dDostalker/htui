#[derive(PartialEq, Eq, Debug, Clone)]
pub enum NodeType {
    None,
    Global,
    Span,
    Div,
    P,
    Text(String),
    // todo!
}

#[derive(Debug)]
pub struct Node {
    pub node_type: NodeType,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new() -> Node {
        Node {
            node_type: NodeType::Global,
            children: vec![],
        }
    }
    pub fn from(node_type: NodeType) -> Node {
        Node {
            node_type,
            children: vec![],
        }
    }

    pub fn text(data: &str) -> Node {
        Node {
            node_type: NodeType::Text(data.to_string()),
            children: vec![],
        }
    }
    pub fn div() -> Node {
        Node {
            node_type: NodeType::Div,
            children: vec![],
        }
    }
}

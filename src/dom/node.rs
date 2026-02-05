type AttrType = Vec<(String, String)>;
pub enum NodeType {
    Document,
    Element,
    Text(String),
    // todo!
}

pub struct ElementData {
    pub tag: String,
    pub attributes: AttrType,
}

pub struct Node {
    pub node_type: NodeType,
    pub children: Vec<Node>,
}

impl Node {
    fn text(data: String) -> Node {
        Node {
            node_type: NodeType::Text(data),
            children: vec![],
        }
    }
    fn element(tag: String, attributes: AttrType, children: Vec<Node>) -> Node {
        Node {
            node_type: NodeType::Element(ElementData { tag, attributes }),
            children,
        }
    }
}

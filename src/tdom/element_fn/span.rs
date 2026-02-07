use anyhow::Result;

use crate::tdom::node::NodeType;

pub fn span_element(element: &str) -> Result<Option<NodeType>> {
    if element.starts_with("<span>") || element.starts_with("</span>") {
        Ok(Some(NodeType::Span))
    } else {
        Ok(None)
    }
}

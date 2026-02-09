use anyhow::Result;

use crate::tdom::node::NodeType;

pub fn p_element(element: &str) -> Result<Option<NodeType>> {
    if element.starts_with("<p>") || element.starts_with("</p>") {
        Ok(Some(NodeType::P))
    } else {
        Ok(None)
    }
}

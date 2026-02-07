use anyhow::Result;

use crate::tdom::node::NodeType;

pub fn div_element(element: &str) -> Result<Option<NodeType>> {
    if element.starts_with("<div>") || element.starts_with("</div>") {
        Ok(Some(NodeType::Div))
    } else {
        Ok(None)
    }
}

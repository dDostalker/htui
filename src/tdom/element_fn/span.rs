use anyhow::Result;

use crate::tdom::{
    attributes::Attr, 
    node::NodeType
};

pub fn span_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<span>") || element.starts_with("</span>") {
        Ok(Some((NodeType::Span, Attr::new())))
    } else {
        Ok(None)
    }
}

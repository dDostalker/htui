use anyhow::Result;

use crate::tdom::{
    attributes::Attr, 
    node::NodeType
};

pub fn p_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<p>") || element.starts_with("</p>") {
        Ok(Some((NodeType::P, Attr::new())))
    } else {
        Ok(None)
    }
}

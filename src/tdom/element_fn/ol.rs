use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn ol_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<ol>") || element.starts_with("</ol>") {
        Ok(Some((NodeType::Ol, Attr::new())))
    } else if element.starts_with("<ol") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<ol").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Ol, attr)))
    } else {
        Ok(None)
    }
}

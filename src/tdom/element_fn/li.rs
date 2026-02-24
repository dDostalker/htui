use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn li_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<li>") || element.starts_with("</li>") {
        Ok(Some((NodeType::Li, Attr::new())))
    } else if element.starts_with("<li") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<li").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Li, attr)))
    } else {
        Ok(None)
    }
}

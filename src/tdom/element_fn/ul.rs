use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn ul_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<ul>") || element.starts_with("</ul>") {
        Ok(Some((NodeType::Ul, Attr::new())))
    } else if element.starts_with("<ul") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<ul").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Ul, attr)))
    } else {
        Ok(None)
    }
}

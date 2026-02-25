use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn p_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<p>") || element.starts_with("</p>") {
        Ok(Some((NodeType::P, Attr::new())))
    } else if element.starts_with("<p") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<p").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::P, attr)))
    } else {
        Ok(None)
    }
}

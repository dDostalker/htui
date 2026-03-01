use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn select_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<select>") || element.starts_with("</select>") {
        Ok(Some((NodeType::Select, Attr::new())))
    } else if element.starts_with("<select") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<select").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Select, attr)))
    } else {
        Ok(None)
    }
}

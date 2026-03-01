use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn summary_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<summary>") || element.starts_with("</summary>") {
        Ok(Some((NodeType::Summary, Attr::new())))
    } else if element.starts_with("<summary") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<summary").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Summary, attr)))
    } else {
        Ok(None)
    }
}

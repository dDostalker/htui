use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn details_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<details>") || element.starts_with("</details>") {
        Ok(Some((NodeType::Details, Attr::new())))
    } else if element.starts_with("<details") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<details").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Details, attr)))
    } else {
        Ok(None)
    }
}

use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn aside_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<aside>") || element.starts_with("</aside>") {
        Ok(Some((NodeType::Aside, Attr::new())))
    } else if element.starts_with("<aside") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<aside").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Aside, attr)))
    } else {
        Ok(None)
    }
}

use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn header_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<header>") || element.starts_with("</header>") {
        Ok(Some((NodeType::Header, Attr::new())))
    } else if element.starts_with("<header") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<header").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Header, attr)))
    } else {
        Ok(None)
    }
}

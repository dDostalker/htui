use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn footer_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<footer>") || element.starts_with("</footer>") {
        Ok(Some((NodeType::Footer, Attr::new())))
    } else if element.starts_with("<footer") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<footer").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Footer, attr)))
    } else {
        Ok(None)
    }
}

use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn button_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<button>") || element.starts_with("</button>") {
        Ok(Some((NodeType::Button, Attr::new())))
    } else if element.starts_with("<button") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<button").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Button, attr)))
    } else {
        Ok(None)
    }
}

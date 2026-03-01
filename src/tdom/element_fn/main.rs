use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn main_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<main>") || element.starts_with("</main>") {
        Ok(Some((NodeType::Main, Attr::new())))
    } else if element.starts_with("<main") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<main").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Main, attr)))
    } else {
        Ok(None)
    }
}

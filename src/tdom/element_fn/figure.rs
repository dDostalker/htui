use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn figure_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<figure>") || element.starts_with("</figure>") {
        Ok(Some((NodeType::Figure, Attr::new())))
    } else if element.starts_with("<figure") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<figure").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Figure, attr)))
    } else {
        Ok(None)
    }
}

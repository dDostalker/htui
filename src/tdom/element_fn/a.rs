use crate::tdom::{attributes::Attr, node::NodeType};
use anyhow::Result;

pub fn a_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<a>") || element.starts_with("</a>") {
        Ok(Some((NodeType::A, Attr::new())))
    } else if element.starts_with("<a") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<a").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Img, attr)))
    } else {
        Ok(None)
    }
}

use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn figcaption_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<figcaption>") || element.starts_with("</figcaption>") {
        Ok(Some((NodeType::Figcaption, Attr::new())))
    } else if element.starts_with("<figcaption") && element.ends_with(">") {
        let attributes_str = element
            .trim_start_matches("<figcaption")
            .trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Figcaption, attr)))
    } else {
        Ok(None)
    }
}

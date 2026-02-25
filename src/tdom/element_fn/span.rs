use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn span_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<span>") || element.starts_with("</span>") {
        Ok(Some((NodeType::Span, Attr::new())))
    } else if element.starts_with("<span") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<span").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Span, attr)))
    } else {
        Ok(None)
    }
}

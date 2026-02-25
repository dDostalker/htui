use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn textarea_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<textarea>") || element.starts_with("</textarea>") {
        Ok(Some((NodeType::Textarea, Attr::new())))
    } else if element.starts_with("<textarea") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<textarea").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Textarea, attr)))
    } else {
        Ok(None)
    }
}

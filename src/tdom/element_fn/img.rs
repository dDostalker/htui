use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn img_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<img") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<img").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Img, attr)))
    } else {
        Ok(None)
    }
}

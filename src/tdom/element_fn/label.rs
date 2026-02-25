use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn label_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<label>") || element.starts_with("</label>") {
        Ok(Some((NodeType::Label, Attr::new())))
    } else if element.starts_with("<label") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<label").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Label, attr)))
    } else {
        Ok(None)
    }
}

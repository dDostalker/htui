use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn section_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<section>") || element.starts_with("</section>") {
        Ok(Some((NodeType::Section, Attr::new())))
    } else if element.starts_with("<section") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<section").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Section, attr)))
    } else {
        Ok(None)
    }
}

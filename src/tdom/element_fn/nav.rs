use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn nav_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<nav>") || element.starts_with("</nav>") {
        Ok(Some((NodeType::Nav, Attr::new())))
    } else if element.starts_with("<nav") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<nav").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Nav, attr)))
    } else {
        Ok(None)
    }
}

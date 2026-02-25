use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn form_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<form>") || element.starts_with("</form>") {
        Ok(Some((NodeType::Form, Attr::new())))
    } else if element.starts_with("<form") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<form").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Form, attr)))
    } else {
        Ok(None)
    }
}

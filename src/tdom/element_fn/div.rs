use anyhow::{Ok, Result};

use crate::tdom::node::NodeType;

pub fn div_element(element: &str) -> Result<Option<NodeType>> {
    if element.starts_with("</div") && element.ends_with(">") {
        Ok(Some(NodeType::Div))
    } else if element.starts_with("<div") && element.ends_with(">") {
        let attributes_strs = element.trim_start_matches("<div").trim_end_matches(">");
        for attributes_str in attributes_strs.split_ascii_whitespace() {
            dbg!(&attributes_str);
            // todo!("Implement attribute parsing")
        }
        Ok(Some(NodeType::Div))
    } else {
        Ok(None)
    }
}

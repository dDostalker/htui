use anyhow::{Ok, Result};

use crate::tdom::{
    attributes::Attr, 
    node::NodeType
};

pub fn div_element(element: &str) -> Result<Option<(NodeType,Attr)>> {
    if element.starts_with("</div") && element.ends_with(">") {
        Ok(Some((NodeType::Div,Attr::new())))
    } else if element.starts_with("<div") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<div").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);  
        Ok(Some((NodeType::Div,attr)))
    } else {
        Ok(None)
    }
}

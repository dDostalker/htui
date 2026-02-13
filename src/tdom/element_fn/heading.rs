use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn h1_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<h1>") || element.starts_with("</h1>") {
        Ok(Some((NodeType::H1, Attr::new())))
    } else if element.starts_with("<h1") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<h1").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::H1, attr)))
    } else {
        Ok(None)
    }
}

pub fn h2_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<h2>") || element.starts_with("</h2>") {
        Ok(Some((NodeType::H2, Attr::new())))
    } else if element.starts_with("<h2") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<h2").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::H2, attr)))
    } else {
        Ok(None)
    }
}

pub fn h3_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<h3>") || element.starts_with("</h3>") {
        Ok(Some((NodeType::H3, Attr::new())))
    } else if element.starts_with("<h3") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<h3").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::H3, attr)))
    } else {
        Ok(None)
    }
}

pub fn h4_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<h4>") || element.starts_with("</h4>") {
        Ok(Some((NodeType::H4, Attr::new())))
    } else if element.starts_with("<h4") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<h4").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::H4, attr)))
    } else {
        Ok(None)
    }
}

pub fn h5_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<h5>") || element.starts_with("</h5>") {
        Ok(Some((NodeType::H5, Attr::new())))
    } else if element.starts_with("<h5") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<h5").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::H5, attr)))
    } else {
        Ok(None)
    }
}

pub fn h6_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<h6>") || element.starts_with("</h6>") {
        Ok(Some((NodeType::H6, Attr::new())))
    } else if element.starts_with("<h6") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<h6").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::H6, attr)))
    } else {
        Ok(None)
    }
}
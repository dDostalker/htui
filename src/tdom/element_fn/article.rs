use anyhow::Result;

use crate::tdom::{attributes::Attr, node::NodeType};

pub fn article_element(element: &str) -> Result<Option<(NodeType, Attr)>> {
    if element.starts_with("<article>") || element.starts_with("</article>") {
        Ok(Some((NodeType::Article, Attr::new())))
    } else if element.starts_with("<article") && element.ends_with(">") {
        let attributes_str = element.trim_start_matches("<article").trim_end_matches(">");
        let attr = Attr::parse_from_str(attributes_str);
        Ok(Some((NodeType::Article, attr)))
    } else {
        Ok(None)
    }
}

use anyhow::Result;

use crate::{
    layout::{layout::TermMap, DEFAULT_LAYOUT},
    tdom::node::{Node, NodeType},
    tui::graph_api::TermLayout,
};

pub fn layout_p(tdom: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()> {
    if tdom.node_type != NodeType::P {
        return Ok(());
    }
    
    // 块级元素开始前换行
    term_layout.new_line();

    for child in tdom.children() {
        match child.node_type.clone() {
            NodeType::Text(s) => {
                let color = tdom.css_attr.get_color();
                term_layout.paint_string(&s, color);
            }
            _ => {
                for layout in DEFAULT_LAYOUT {
                    layout(child, term_layout, term_map)?;
                }
            }
        }
    }
    
    // 块级元素结束后换行
    term_layout.new_line();
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{html::html_parse::Parser, layout::layout::TermMap, tui::graph_api::TermLayout};

    #[test]
    fn test_layout_p() {
        let html = "
                <p>Hello</p>
                <p>World</p>
            ";
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();

        let term_map = TermMap::get_ter_map().unwrap();
        let mut term_layout = TermLayout::new(&term_map).unwrap();
        term_layout.draw(&node, &term_map).unwrap();
        dbg!(term_layout.layout);
    }
}

use anyhow::Result;

use crate::{
    layout::layout::TermMap,
    tdom::node::{Node, NodeType},
    tui::graph_api::TermLayout,
};

pub fn layout_p(tdom: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()> {
    for child in tdom.children() {
        match child.node_type.clone() {
            NodeType::Text(s) => {
                let column = term_map.columns;

                term_layout.paint_string(&s, None);
                let len = term_layout.len;
                let yu = column as u64 - len % column as u64;
                let point = &mut term_layout.point;
                *point += yu;
            }

            _ => {}
        }
    }
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

use anyhow::Result;

use crate::{
    layout::layout::{TermLayout, TermMap},
    tdom::node::{Node, NodeType},
};

pub fn layout_p(tdom: &Node, term_layout: &mut TermLayout, term_map: &TermMap) -> Result<()> {
    for child in tdom.children() {
        match child.node_type.clone() {
            NodeType::Text(s) => {
                let mut point = term_layout.point;
                let column = term_map.columns;
                let row = term_map.rows;
                let len = s.chars().count();
                for c in s.chars() {
                    term_layout.layout[point as usize] = c;
                    point += 1;
                }
                let yu = column as u64 - point % column as u64;
                for _ in 0..yu {
                    term_layout.layout[point as usize] = ' ';
                    point += 1;
                }
                term_layout.point = point;
            }

            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        html::html_parse::Parser,
        layout::layout::{TermLayout, TermMap},
    };

    #[test]
    fn test_layout_p() {
        let html = "
                <p>Hello</p>
                <p>World</p>
            ";
        let mut p = Parser::new(html);
        let node = p.parse().unwrap();

        let mut term_map = TermMap::get_ter_map().unwrap();
        let mut term_layout = TermLayout::new(&term_map).unwrap();
        term_layout.draw(&node, &term_map);
        dbg!(term_layout.layout);
    }
}

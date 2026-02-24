use anyhow::Result;
use htui::{Parser, TermLayout, TermMap};

fn main() -> Result<()> {
    let html = "
             <p>Hello</p>
             <p>World</p>
         ";
    let mut p = Parser::new(html);
    let node = p.parse()?;

    let term_map = TermMap::get_ter_map()?;
    let mut term_layout = TermLayout::new(&term_map)?;
    term_layout.draw(&node, &term_map)?;

    println!("{}", term_layout);
    Ok(())
}

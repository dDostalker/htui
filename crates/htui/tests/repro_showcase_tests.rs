//! Temporary repro: showcase dashboard overflow at small sizes.

use htui::{
    html::{css_parse, html_parse},
    layout::layout::TermMap,
    layout::LayoutBox,
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::TermLayout,
};

fn setup(html: &str) -> (NodeId, NodeArena) {
    let mut parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = parser.parse();
    if !embedded.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&embedded);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    (root_id, arena)
}

fn walk(b: &LayoutBox, arena: &NodeArena, depth: usize, out: &mut Vec<String>) {
    let tag = arena[b.node_id].node_type.tag_name().unwrap_or("?").to_string();
    out.push(format!(
        "{}{} x={} y={} w={} h={}",
        "  ".repeat(depth),
        tag,
        b.rect.x,
        b.rect.y,
        b.rect.width,
        b.rect.height
    ));
    for c in &b.children {
        walk(c, arena, depth + 1, out);
    }
}

#[test]
fn repro_showcase() {
    let html = include_str!("../../../visual_test/src/showcase/index.html");
    let (root_id, arena) = setup(html);
    let map = TermMap { columns: 60, rows: 30 };
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);
    let boxes = tl.layout_only(&arena, root_id, &map);
    let mut tree = Vec::new();
    for b in &boxes {
        walk(b, &arena, 0, &mut tree);
    }
    for l in &tree {
        println!("{l}");
    }
}

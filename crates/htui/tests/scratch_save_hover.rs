//! Scratch: inspect configurator save/reset button geometry + hover paint.

use htui::events::{EventBindings, HitMap};
use htui::html::css_parse::{css_into_node, css_into_node_with_hover, reset_css_attr};
use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::layout::{BoxConstraint, Cursor, Rect, layout_tree};
use htui::script::ScriptEngine;
use htui::tui::graph_api::TermLayout;

const COLS: u16 = 104;
const ROWS: u16 = 32;

fn find_box<'a>(boxes: &'a [htui::layout::LayoutBox], node_id: htui::tdom::node::NodeId) -> Option<&'a htui::layout::LayoutBox> {
    for b in boxes {
        if b.node_id == node_id {
            return Some(b);
        }
        if let Some(f) = find_box(&b.children, node_id) {
            return Some(f);
        }
    }
    None
}

fn dump(tl: &TermLayout, y0: usize, y1: usize, label: &str) {
    let cols = tl.columns as usize;
    let hover = crossterm::style::Color::Rgb { r: 0x28, g: 0x30, b: 0x4d };
    println!("--- {label} ---");
    for y in y0..y1 {
        let mut s = String::new();
        let mut bg = String::new();
        for x in 0..cols {
            let idx = y * cols + x;
            if idx >= tl.layout.len() {
                break;
            }
            let c = &tl.layout[idx];
            s.push(if c.word.is_empty() { '·' } else { c.word.chars().next().unwrap() });
            bg.push(match c.bcolor {
                Some(col) if col == hover => 'H',
                Some(_) => '█',
                None => ' ',
            });
        }
        println!("{y:02}|{s}");
        println!("bg |{bg}");
    }
}

#[test]
fn scratch_configurator_save_reset_geometry() {
    let html = include_str!("../../../visual_test/src/configurator/index.html");
    let mut engine = ScriptEngine::new(html, EventBindings::default()).expect("engine");
    let mut arena = engine.arena();
    let root_id = engine.root_id();
    let mut parser = html_parse::Parser::new(html);
    let (_r, _a, embedded) = parser.parse();
    let mut css_parser = css_parse::Parser::new(&embedded);
    let rules = css_parser.parse();
    css_parse::css_into_node(&mut arena, root_id, &rules);

    let term_map = TermMap { columns: COLS, rows: ROWS };
    let containing = BoxConstraint::root(&term_map);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, containing, &mut cursor, COLS as usize, ROWS as usize);
    let map = HitMap::build(&htui::events::Geometry { arena: arena.clone(), boxes: boxes.clone(), root_id });
    let save_region = map.resolve_by_id("save").expect("save").clone();
    let reset_region = map.resolve_by_id("reset").expect("reset").clone();
    let status_region = map.resolve_by_id("status").expect("status").clone();
    let save_node = arena.iter().find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("save")).map(|(i, _)| i).unwrap();
    let reset_node = arena.iter().find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("reset")).map(|(i, _)| i).unwrap();
    let save_box = find_box(&boxes, save_node).unwrap();
    let reset_box = find_box(&boxes, reset_node).unwrap();
    println!("save box rect {:?} fill_bg={} chrome={}", save_box.rect, save_box.paint_style.fill_bg, save_box.paint_style.chrome);
    println!("reset box rect {:?}", reset_box.rect);
    println!("save hit-region rect {:?}", save_region.rect);
    println!("reset hit-region rect {:?}", reset_region.rect);
    println!("status hit-region rect {:?}", status_region.rect);

    // Also dump the rows around the buttons for a base (unhovered) paint.
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.repaint_from(&arena, &boxes);
    dump(&tl, 44, 59, "base wide");

    // Hover over save: repaint only the hover-affected region.
    let mut arena_h = arena.clone();
    reset_css_attr(&mut arena_h, root_id);
    css_into_node_with_hover(&mut arena_h, root_id, &rules, Some(save_node));
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.repaint_from(&arena_h, &boxes);
    dump(&tl, save_box.rect.y as usize - 1, save_box.rect.y as usize + save_box.rect.height as usize + 2, "hover save (full repaint)");

    // Region repaint variant (what the Hover handler actually does).
    let region = save_box.rect;
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.repaint_from(&arena, &boxes);
    let mut tl2 = TermLayout::new(&term_map, 1.0);
    tl2.repaint_from(&arena, &boxes);
    tl2.repaint_region(&arena_h, &boxes, region);
    dump(&tl2, save_box.rect.y as usize - 1, save_box.rect.y as usize + save_box.rect.height as usize + 2, "hover save (region repaint)");

    // Inspect the actual diff bytes the terminal receives, scrolled so the
    // buttons are inside the viewport (like the real app when scrolled down).
    let mut t3 = TermLayout::new(&term_map, 1.0);
    t3.repaint_from(&arena, &boxes);
    t3.scroll_y = 27;
    let first = t3.render_diff_to_bytes();
    println!("first frame bytes len: {}", first.len());
    t3.repaint_region(&arena_h, &boxes, region);
    let bytes = t3.render_diff_to_bytes();
    let s = String::from_utf8_lossy(&bytes);
    println!("--- diff bytes after hover repaint (len {}) ---", bytes.len());
    println!("{s:?}");

    // What the hit test sees at each row of the button.
    for dy in 0..save_box.rect.height {
        let y = save_box.rect.y + dy;
        let x = save_box.rect.x + save_box.rect.width / 2;
        let r = map.hit_test(x as u32, y as u32);
        let id = r.map(|r| r.node_id);
        let name = id.and_then(|i| arena.get(i).and_then(|n| n.get_id().cloned())).unwrap_or_default();
        println!("hit at y={y} (dy={dy}) -> {name}");
    }

    // The `.row` flex container box + hit tests just above the buttons.
    let mut cur = Some(save_node);
    while let Some(id) = cur {
        if let Some(node) = arena.get(id) {
            if let Some(pb) = find_box(&boxes, id) {
                let cls = node.attributes.get("class").cloned().unwrap_or_default();
                println!("ancestor rect {:?} class={:?} id={:?}", pb.rect, cls, node.get_id().cloned());
            }
            cur = node.parent;
        } else {
            cur = None;
        }
    }
    let mut by = save_box.rect.y;
    while by > 0 {
        by -= 1;
        let x = 5u32;
        let r = map.hit_test(x, by as u32);
        let id = r.map(|r| r.node_id);
        let name = id.and_then(|i| arena.get(i).and_then(|n| n.get_id().cloned())).unwrap_or_default();
        println!("hit above at y={by} -> {name}");
        if by == 0 {
            break;
        }
        if by + 4 < save_box.rect.y {
            break;
        }
    }
}

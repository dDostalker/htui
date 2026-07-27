//! Box-model visual test inspection.
//!
//! Renders the actual `visual_test/src/box_model/index.html` and
//! `visual_test/src/all_in_one/index.html` fixtures to a mock terminal and
//! dumps a row-by-row snapshot so bg / padding / border / margin alignment
//! can be checked without a real TTY. Run with `--nocapture` to see the
//! rendered canvas:
//!
//! ```text
//! cargo test --test layout_box_model_debug_tests -- --nocapture
//! ```

use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tui::graph_api::TermLayout;
use crossterm::style::Color as CtColor;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

const COLS: u16 = 120;
const ROWS: u16 = 60;

// Run a separate test with a much wider canvas to confirm the block geometry
// holds up at non-typical widths (e.g. 200+).

fn mock_term_map() -> TermMap {
    TermMap { columns: COLS, rows: ROWS }
}

fn draw(html: &str, css: &str) -> (TermMap, TermLayout) {
    let term_map = mock_term_map();
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    (term_map, tl)
}

fn bg_marker(c: Option<CtColor>) -> char {
    match c {
        Some(CtColor::DarkBlue) => 'B',
        Some(CtColor::DarkGreen) => 'g',
        Some(CtColor::DarkRed) => 'r',
        Some(CtColor::DarkCyan) => 'c',
        Some(CtColor::DarkYellow) => 'y',
        Some(CtColor::DarkMagenta) => 'm',
        Some(CtColor::Black) => 'K',
        Some(CtColor::Grey) => 'G',
        Some(CtColor::Cyan) => 'C',
        Some(CtColor::Green) => 'G',
        Some(CtColor::Yellow) => 'Y',
        Some(CtColor::Red) => 'R',
        Some(CtColor::White) => 'W',
        Some(CtColor::Magenta) => 'M',
        _ => '.',
    }
}

fn dump_html(label: &str, html: &str) {
    let (term_map, tl) = draw(html, "");
    let cols = term_map.columns as usize;
    let total = (tl.rendered_rows as usize).min(ROWS as usize);
    eprintln!("---- {label} ({total} rows) ----");
    for r in 0..total {
        let s = r * cols;
        let mut line = String::new();
        for c in 0..cols {
            let b = &tl.layout[s + c];
            let marker = bg_marker(b.bcolor);
            if !b.word.is_empty() && b.word != " " {
                line.push_str(&b.word);
            } else if marker != '.' {
                line.push(marker);
            } else {
                line.push('.');
            }
        }
        eprintln!("row {r:>2}: {line}");
    }
}

#[test]
fn dump_visual_box_model() {
    init();
    let html = include_str!("../visual_test/src/box_model/index.html");
    dump_html("visual_test/box_model", html);
}

#[test]
fn dump_visual_all_in_one() {
    init();
    let html = include_str!("../visual_test/src/all_in_one/index.html");
    dump_html("visual_test/all_in_one", html);
}

#[test]
fn dump_visual_font_size() {
    init();
    let html = include_str!("../visual_test/src/font_size/index.html");
    dump_html("visual_test/font_size", html);
}

#[test]
fn dump_font_size_wide_canvas() {
    init();
    use htui::html::html_parse;
    use htui::layout::{BoxConstraint, Cursor};
    use htui::layout::tree::layout_tree;
    let html = include_str!("../visual_test/src/font_size/index.html");
    let term_map = TermMap { columns: 240, rows: 60 };
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    if !embedded.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&embedded);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    let mut tl = TermLayout::new(&term_map, 1.0);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, BoxConstraint::root(&term_map), &mut cursor);
    fn walk(boxes: &[htui::layout::LayoutBox], depth: usize) {
        for b in boxes {
            let indent = " ".repeat(depth);
            let runs_dbg: Vec<String> = b.runs.as_ref().map(|rs| {
                rs.iter().map(|r| format!("text={:?} bg={:?}", r.text, r.bg)).collect()
            }).unwrap_or_default();
            eprintln!(
                "{indent}node={:?} rect=({},{},w={},h={}) kind={:?} fs={} pad=t{}r{}b{}l{} border=t{}r{}b{}l{} mar=t{}r{}b{}l{} runs={:?}",
                b.node_id,
                b.rect.x, b.rect.y, b.rect.width, b.rect.height,
                b.kind,
                b.font_size_cells,
                b.box_edges.padding.top, b.box_edges.padding.right, b.box_edges.padding.bottom, b.box_edges.padding.left,
                b.box_edges.border.top.thickness(), b.box_edges.border.right.thickness(),
                b.box_edges.border.bottom.thickness(), b.box_edges.border.left.thickness(),
                b.box_edges.margin.top, b.box_edges.margin.right, b.box_edges.margin.bottom, b.box_edges.margin.left,
                runs_dbg,
            );
            walk(&b.children, depth + 1);
        }
    }
    walk(&boxes, 0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    let cols = term_map.columns as usize;
    let total = (tl.rendered_rows as usize).min(term_map.rows as usize);
    eprintln!("---- font_size @ 240 cols ----");
    for r in 28..=42 {
        if r >= total {
            break;
        }
        let s = r * cols;
        let mut line = String::new();
        for c in 0..cols {
            let b = &tl.layout[s + c];
            let marker = bg_marker(b.bcolor);
            if !b.word.is_empty() && b.word != " " {
                line.push_str(&b.word);
            } else if marker != '.' {
                line.push(marker);
            } else {
                line.push('.');
            }
        }
        eprintln!("row {r:>2}: {} |len={}|", line, line.len());
    }
}

#[test]
fn dump_pad_em_text_node() {
    init();
    use htui::html::html_parse;
    let html = include_str!("../visual_test/src/font_size/index.html");
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, arena, _embedded) = html_parser.parse();
    fn walk(arena: &htui::tdom::node::NodeArena, id: htui::tdom::node::NodeId, depth: usize) {
        let node = &arena[id];
        let indent = " ".repeat(depth);
        match &node.node_type {
            htui::tdom::node::NodeType::Text(s) => {
                eprintln!("{indent}Text(len={}, content={:?})", s.len(), s);
            }
            _ => {
                eprintln!("{indent}{:?}", node.node_type);
                for child_id in node.child_ids() {
                    walk(arena, child_id, depth + 1);
                }
            }
        }
    }
    walk(&arena, root_id, 0);
    // Find the .pad-em element and dump its text
    eprintln!("---- searching for pad-em ----");
    fn find_pad(arena: &htui::tdom::node::NodeArena, id: htui::tdom::node::NodeId, depth: usize) {
        let node = &arena[id];
        let indent = " ".repeat(depth);
        let class = node.css_attr.0.get("class").cloned().unwrap_or_default();
        if class.contains("pad") || class.contains("em") {
            eprintln!("{indent}class={:?} attrs={:?}", class, node.css_attr.0);
        }
        for child_id in node.child_ids() {
            find_pad(arena, child_id, depth + 1);
        }
    }
    find_pad(&arena, root_id, 0);
}

#[test]
fn dump_visual_font_size_per_cell() {
    init();
    let html = r##"<!DOCTYPE html>
<html><head><style>
.pad-em { font-size: 2em; padding: 0.5em 1em; background-color: darkgreen; color: white; }
</style></head><body><div><p class="pad-em">em padding</p></div></body></html>
"##;
    let (term_map, tl) = draw(html, "");
    let cols = term_map.columns as usize;
    eprintln!("---- per-cell dump (width {cols}) ----");
    for r in 0..4 {
        let s = r * cols;
        eprintln!("row {r}:");
        for c in 0..30 {
            let b = &tl.layout[s + c];
            eprintln!("  col {c:>3}: word={:?} bg={:?}", b.word, b.bcolor);
        }
    }
}

#[test]
fn dump_full_font_size_per_cell() {
    init();
    let html = include_str!("../visual_test/src/font_size/index.html");
    let (term_map, tl) = draw(html, "");
    let cols = term_map.columns as usize;
    eprintln!("---- full font_size per-cell dump ----");
    for r in 31..=34 {
        let s = r * cols;
        eprintln!("row {r}:");
        for c in 0..cols {
            let b = &tl.layout[s + c];
            if b.bcolor.is_some() || !b.word.is_empty() {
                eprintln!("  col {c:>3}: word={:?} bg={:?}", b.word, b.bcolor);
            }
        }
    }
}

#[test]
fn dump_visual_font_size_with_markers() {
    init();
    let (term_map, tl) = draw(
        include_str!("../visual_test/src/font_size/index.html"),
        "",
    );
    let cols = term_map.columns as usize;
    eprintln!("---- cols={cols} layout_len={} rendered_rows={} ----", tl.layout.len(), tl.rendered_rows);
    let total = (tl.rendered_rows as usize).min(ROWS as usize);
    // Build a column ruler.
    let ruler1 = (0..cols).map(|c| (b'0' + (c % 10) as u8) as char).collect::<String>();
    let ruler2 = (0..(cols / 10)).map(|c| char::from(b'0' + c as u8)).collect::<String>();
    eprintln!("       {}", ruler2);
    eprintln!("       {}", ruler1);
    for r in 28..=42 {
        if r >= total {
            break;
        }
        let s = r * cols;
        let mut line = String::new();
        for c in 0..cols {
            let b = &tl.layout[s + c];
            let marker = bg_marker(b.bcolor);
            if !b.word.is_empty() && b.word != " " {
                line.push_str(&b.word);
            } else if marker != '.' {
                line.push(marker);
            } else {
                line.push('.');
            }
        }
        eprintln!("row {r:>2}: {} |len={}| bytes={}", line, line.len(), line.bytes().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
        if r == 32 {
            eprintln!("    ---- chars 0-25 of line ----");
            for (i, ch) in line.chars().take(25).enumerate() {
                eprintln!("    line_char[{i}] = {ch:?} (u+{:04x})", ch as u32);
            }
        }
        // Also dump specific cell info for row 32 cols 0..30.
        if r == 32 {
            eprintln!("    ---- per-cell visual representation ----");
            let chars: Vec<char> = line.chars().collect();
            for c in 0..30 {
                let b = &tl.layout[s + c];
                let marker = bg_marker(b.bcolor);
                let v = if !b.word.is_empty() && b.word != " " {
                    b.word.clone()
                } else if marker != '.' {
                    marker.to_string()
                } else {
                    ".".to_string()
                };
                let line_char = chars.get(c).copied().map(|x| x.to_string()).unwrap_or_else(|| "<oob>".into());
                eprintln!("    col {c:>3}: cell.value={v:?} | line_char={line_char:?} | line.len={} chars.len={}", line.len(), chars.len());
            }
        }
    }
}

#[test]
fn dump_layout_for_pad_em() {
    init();
    use htui::html::html_parse;
    use htui::layout::{BoxConstraint, Cursor};
    use htui::layout::tree::layout_tree;
    use htui::layout::paint::paint_layout_tree;

    let html = r##"<!DOCTYPE html>
<html><head><style>
.pad-em { font-size: 2em; padding: 0.5em 1em; background-color: darkgreen; color: white; }
</style></head><body><div><p class="pad-em">em padding</p></div></body></html>
"##;
    let term_map = TermMap { columns: COLS, rows: ROWS };
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    let combined = if embedded.is_empty() { String::new() } else { embedded };
    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    let mut tl = TermLayout::new(&term_map, 1.0);
    let mut cursor = Cursor::new();
    let boxes = layout_tree(&arena, root_id, BoxConstraint::root(&term_map), &mut cursor);
    fn walk(boxes: &[htui::layout::LayoutBox], depth: usize) {
        for b in boxes {
            let indent = " ".repeat(depth);
            let runs_dbg: Vec<String> = b.runs.as_ref().map(|rs| {
                rs.iter().map(|r| format!("text={:?} bg={:?}", r.text, r.bg)).collect()
            }).unwrap_or_default();
            eprintln!(
                "{indent}node={:?} rect=({},{},w={},h={}) kind={:?} fs={} pad=t{}r{}b{}l{} border=t{}r{}b{}l{} mar=t{}r{}b{}l{} runs={:?}",
                b.node_id,
                b.rect.x, b.rect.y, b.rect.width, b.rect.height,
                b.kind,
                b.font_size_cells,
                b.box_edges.padding.top, b.box_edges.padding.right, b.box_edges.padding.bottom, b.box_edges.padding.left,
                b.box_edges.border.top.thickness(), b.box_edges.border.right.thickness(),
                b.box_edges.border.bottom.thickness(), b.box_edges.border.left.thickness(),
                b.box_edges.margin.top, b.box_edges.margin.right, b.box_edges.margin.bottom, b.box_edges.margin.left,
                runs_dbg,
            );
            walk(&b.children, depth + 1);
        }
    }
    walk(&boxes, 0);
    paint_layout_tree(&boxes, &arena, &mut tl);
    let cols = term_map.columns as usize;
    let total = (tl.rendered_rows as usize).min(ROWS as usize);
    eprintln!("---- pad-em test ----");
    for r in 0..total {
        let s = r * cols;
        let mut line = String::new();
        for c in 0..cols {
            let b = &tl.layout[s + c];
            let marker = bg_marker(b.bcolor);
            if !b.word.is_empty() && b.word != " " {
                line.push_str(&b.word);
            } else if marker != '.' {
                line.push(marker);
            } else {
                line.push('.');
            }
        }
        eprintln!("row {r:>2}: {line}");
    }
}

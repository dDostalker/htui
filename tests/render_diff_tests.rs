use htui::{
    html::{css_parse, html_parse},
    layout::layout::TermMap,
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::TermLayout,
};

fn setup(html: &str) -> (NodeId, NodeArena) {
    let mut html_parser = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = html_parser.parse();
    if !embedded.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&embedded);
        let rules = css_parser.parse();
        css_parse::css_into_node(&mut arena, root_id, &rules);
    }
    (root_id, arena)
}

fn term_map() -> TermMap {
    TermMap {
        columns: 80,
        rows: 24,
    }
}

#[test]
fn unchanged_frame_emits_zero_bytes() {
    let (root_id, arena) = setup("<div>hello world</div>");
    let map = term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map).unwrap();

    let first = tl.render_diff_to_bytes();
    assert!(!first.is_empty(), "first frame must emit content");
    let first_text = String::from_utf8_lossy(&first);
    assert!(
        first_text.contains("hello") && first_text.contains("world"),
        "first frame must contain the document text"
    );

    let second = tl.render_diff_to_bytes();
    assert!(second.is_empty(), "unchanged frame must emit nothing");
}

#[test]
fn changed_content_emits_smaller_diff_than_full_frame() {
    let (root_id, arena) = setup("<div>first</div>");
    let map = term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map).unwrap();

    let full = tl.render_to_bytes();
    assert!(String::from_utf8_lossy(&full).contains("first"));
    let first_diff = tl.render_diff_to_bytes();
    assert!(!first_diff.is_empty());
    assert!(
        first_diff.len() < full.len(),
        "first diff must be smaller than the whole-viewport frame"
    );

    // A new document with different text at the same position.
    let (root2, arena2) = setup("<div>second</div>");
    tl.draw(&arena2, root2, &map).unwrap();
    let diff = tl.render_diff_to_bytes();
    assert!(!diff.is_empty(), "changed frame must emit something");
    assert!(diff.len() < full.len(), "diff must stay below full-frame size");
    assert!(
        String::from_utf8_lossy(&diff).contains("second"),
        "diff must carry the new text"
    );
}

#[test]
fn scrolling_emits_diff_and_stable_views_emit_nothing() {
    let mut html = String::from("<div>");
    for i in 0..60 {
        html.push_str(&format!("<p>line {i}</p>"));
    }
    html.push_str("</div>");
    let (root_id, arena) = setup(&html);
    let map = term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map).unwrap();

    assert!(
        !tl.render_diff_to_bytes().is_empty(),
        "first frame must emit content"
    );
    assert!(
        tl.render_diff_to_bytes().is_empty(),
        "stable view must emit nothing"
    );

    tl.scroll_y = 1;
    tl.clamp_scroll();
    assert!(
        tl.max_scroll() >= 1,
        "tall document must be scrollable for this test"
    );
    let scrolled = tl.render_diff_to_bytes();
    assert!(!scrolled.is_empty(), "scrolled frame must emit a diff");
    assert!(
        tl.render_diff_to_bytes().is_empty(),
        "stable scrolled view must emit nothing"
    );
}

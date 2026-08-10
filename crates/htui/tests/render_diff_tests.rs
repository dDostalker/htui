use crossterm::style::Color;
use htui::{
    html::{css_parse, html_parse},
    layout::layout::TermMap,
    tdom::node::{Node, NodeArena, NodeId, NodeType},
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

/// Find the first node carrying `id="target"`.
fn find_id(arena: &NodeArena, target: &str) -> Option<NodeId> {
    fn walk(arena: &NodeArena, id: NodeId, target: &str) -> Option<NodeId> {
        let node = &arena[id];
        if node.attributes.get("id").is_some_and(|v| v == target) {
            return Some(id);
        }
        node
            .children
            .iter()
            .find_map(|&c| walk(arena, c, target))
    }
    let root = arena
        .iter()
        .find_map(|(id, n)| (n.node_type == NodeType::Global).then_some(id));
    root.and_then(|r| walk(arena, r, target))
}

fn term_map() -> TermMap {
    TermMap {
        columns: 80,
        rows: 24,
    }
}

#[test]
fn fresh_canvas_max_scroll_requires_the_paint_pass() {
    // The render thread publishes `max_scroll` right after `layout_only`, but
    // `rendered_rows` (its basis) is only established by the paint pass — a
    // fresh canvas (first frame / every resize) starts at 0. Publishing that
    // 0 would clamp every scroll position and make the wheel / PageUp /
    // PageDown appear dead, so the publication must happen after painting.
    // This test pins down the pre/post-paint difference the ordering fix
    // depends on.
    let mut html = String::from("<div>");
    for i in 0..60 {
        html.push_str(&format!("<p>line {i}</p>"));
    }
    html.push_str("</div>");
    let (root_id, arena) = setup(&html);
    let map = term_map();

    // Before the paint pass a brand-new canvas reports no content height.
    let mut pre = TermLayout::new(&map, 1.0);
    pre.layout_only(&arena, root_id, &map);
    assert_eq!(
        pre.max_scroll(),
        0,
        "pre-paint max_scroll on a fresh canvas must be 0 (this is the value \
         the render thread used to publish, killing the wheel after a resize)"
    );

    // After the full draw the content height is known and scrollable.
    let mut post = TermLayout::new(&map, 1.0);
    post.draw(&arena, root_id, &map);
    assert!(
        post.max_scroll() > 0,
        "post-paint max_scroll must reflect the document height"
    );
}

#[test]
fn scroll_bound_never_exceeds_the_canvas_buffer() {
    // The canvas grows to the full content height (no screen cap), so a
    // normally-painted document always has `rendered_rows <= canvas rows`.
    // Two invariants protect the incremental diff renderer (which slices
    // `layout[scroll_start..]`): the advertised scroll bound never points
    // past the buffer end, and rendering at an out-of-range position is a
    // safe no-op instead of an underflow panic (the old `max_scroll` once
    // advertised the content height for buffer-capped documents and the
    // render thread died on scroll — the `image` visual test reproduced it).
    let mut html = String::from("<div>");
    for _ in 0..120 {
        html.push_str("<div>r</div>");
    }
    html.push_str("</div>");
    let (root_id, arena) = setup(&html);
    let map = TermMap { columns: 10, rows: 4 };
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);

    let canvas_rows = tl.layout.len() / map.columns as usize;
    // The advertised scroll bound stays inside the buffer.
    assert!(
        tl.max_scroll() + map.rows as u32 <= canvas_rows as u32,
        "max_scroll must not point past the canvas end"
    );

    // Clamping a wild scroll position must land inside the buffer.
    tl.scroll_y = u32::MAX;
    tl.clamp_scroll();
    assert!(
        (tl.scroll_y as usize).saturating_mul(map.columns as usize) <= tl.layout.len(),
        "clamped scroll must fit in the canvas"
    );

    // Even an out-of-range position (what the input loop used to send before
    // the bound fix) must render as a safe no-op instead of panicking.
    tl.scroll_y = 1000;
    tl.render_diff_to_bytes();
    tl.render_to_bytes();

    // Simulate a stale oversized content-height report (`rendered_rows` is a
    // monotonic high-water mark; a re-layout after a terminal grow can leave
    // it above the canvas for one frame): the guards must still keep
    // rendering safe.
    tl.rendered_rows = (canvas_rows + 100) as u32;
    tl.scroll_y = 1000;
    tl.render_diff_to_bytes();
    tl.render_to_bytes();
}

#[test]
fn unchanged_frame_emits_zero_bytes() {
    let (root_id, arena) = setup("<div>hello world</div>");
    let map = term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);

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
    tl.draw(&arena, root_id, &map);

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
    tl.draw(&arena2, root2, &map);
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
    tl.draw(&arena, root_id, &map);

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

/// Find the layout box for a node, searching the whole forest.
fn find_box<'a>(
    boxes: &'a [htui::layout::LayoutBox],
    node_id: NodeId,
) -> Option<&'a htui::layout::LayoutBox> {
    for b in boxes {
        if b.node_id == node_id {
            return Some(b);
        }
        if let Some(found) = find_box(&b.children, node_id) {
            return Some(found);
        }
    }
    None
}

#[test]
fn region_repaint_emits_only_the_changed_text() {
    let (root_id, mut arena) = setup("<div><p id=\"a\">hello</p><p id=\"b\">world</p></div>");
    let map = term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);
    let first = tl.render_diff_to_bytes();
    assert!(
        String::from_utf8_lossy(&first).contains("world"),
        "first frame must contain both paragraphs"
    );

    // Simulate `textContent` on `<p id=a>`: rewrite its single text child to
    // a same-width string so the layout (box rects) stays identical.
    let a_id = find_id(&arena, "a").expect("paragraph a");
    let text_child = arena[a_id].children[0];
    if let NodeType::Text(t) = &mut arena[text_child].node_type {
        *t = "helpx".to_string();
    }

    // The incremental pipeline the render thread runs: re-layout, then
    // repaint only the changed box's rect.
    let boxes = tl.layout_only(&arena, root_id, &map);
    let rect = find_box(&boxes, a_id).expect("a's layout box").rect;
    tl.repaint_region(&arena, &boxes, rect);

    let diff = tl.render_diff_to_bytes();
    let text = String::from_utf8_lossy(&diff);
    assert!(text.contains("helpx"), "region repaint must show the new text");
    assert!(
        !text.contains("world"),
        "unchanged sibling must not be re-emitted by a region repaint"
    );
}

#[test]
fn region_repaint_preserves_ancestor_background() {
    let html = concat!(
        "<style>div { background-color: cyan }</style>",
        "<div><p id=\"a\">one</p><p id=\"b\">two</p></div>"
    );
    let (root_id, mut arena) = setup(html);
    let map = term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);
    let first = tl.render_diff_to_bytes();
    assert!(!first.is_empty());

    let a_id = find_id(&arena, "a").expect("paragraph a");
    let text_child = arena[a_id].children[0];
    if let NodeType::Text(t) = &mut arena[text_child].node_type {
        *t = "uno".to_string();
    }

    let boxes = tl.layout_only(&arena, root_id, &map);
    let rect = find_box(&boxes, a_id).expect("a's layout box").rect;
    tl.repaint_region(&arena, &boxes, rect);
    let diff = tl.render_diff_to_bytes();
    assert!(
        String::from_utf8_lossy(&diff).contains("uno"),
        "region repaint must show the new text"
    );

    // The `<p>` itself has no background; its cells must inherit the
    // ancestor `<div>`'s fill through the repaint's parent-bg chain.
    let cols = tl.columns as usize;
    let text_cell = &tl.layout[rect.y.saturating_mul(cols).saturating_add(rect.x)];
    assert_eq!(
        text_cell.bcolor,
        Some(Color::Rgb { r: 0, g: 255, b: 255 }),
        "region repaint must keep the ancestor background on text cells"
    );
}

#[test]
fn region_repaint_clears_stale_cells_of_a_shorter_value() {
    // A fixed-width box whose value shrinks: the trailing cells must be
    // blanked (they were cleared with the region), not keep the old glyphs.
    let html = concat!(
        "<style>p { width: 20px; background-color: yellow }</style>",
        "<p id=\"a\">longvalue</p>"
    );
    let (root_id, mut arena) = setup(html);
    let map = term_map();
    let mut tl = TermLayout::new(&map, 1.0);
    tl.draw(&arena, root_id, &map);
    let first = tl.render_diff_to_bytes();
    assert!(String::from_utf8_lossy(&first).contains("longvalue"));

    let a_id = find_id(&arena, "a").expect("paragraph a");
    let text_child = arena[a_id].children[0];
    if let NodeType::Text(t) = &mut arena[text_child].node_type {
        *t = "hi".to_string();
    }

    let boxes = tl.layout_only(&arena, root_id, &map);
    let rect = find_box(&boxes, a_id).expect("a's layout box").rect;
    tl.repaint_region(&arena, &boxes, rect);
    let diff = tl.render_diff_to_bytes();
    let text = String::from_utf8_lossy(&diff);
    assert!(text.contains("hi"), "region repaint must show the new text");
    assert!(
        !text.contains("longvalue"),
        "stale cells of the previous value must be cleared"
    );
}

#[test]
fn block_element_char_width() {
    // U+2588 FULL BLOCK / U+2591 LIGHT SHADE are the progress-bar glyphs in
    // the sysmon demo. Their East Asian Width is "Ambiguous": Unicode treats
    // them as one column, but CJK-configured legacy terminals may render two.
    // The engine must lay them out consistently with the terminal or the bar
    // misaligns (over-wide row, wrapped percentage, shifted digits).
    use htui::tui::graph_api::{char_width, grapheme_width};
    assert_eq!(grapheme_width("█"), char_width('█'));
    assert_eq!(grapheme_width("░"), char_width('░'));
    assert_eq!(grapheme_width("█"), 1, "full block must be single-width");
    assert_eq!(grapheme_width("░"), 1, "light shade must be single-width");
}

#[test]
fn canvas_starts_at_one_viewport_and_grows_on_demand() {
    let map = term_map(); // 80×24
    let viewport = map.columns as usize * map.rows as usize;
    let mut tl = TermLayout::new(&map, 1.0);
    assert_eq!(
        tl.layout.len(),
        viewport,
        "canvas must start at a single viewport, not the full scroll capacity"
    );

    // A short document must not grow the buffer.
    let (root_id, arena) = setup("<div>hi</div>");
    tl.draw(&arena, root_id, &map);
    assert_eq!(
        tl.layout.len(),
        viewport,
        "short doc must keep the 1-viewport canvas"
    );

    // A tall document grows the buffer exactly to its content height.
    let mut html = String::from("<div>");
    for i in 0..60 {
        html.push_str(&format!("<p>line {i}</p>"));
    }
    html.push_str("</div>");
    let (root_id, arena) = setup(&html);
    tl.draw(&arena, root_id, &map);
    let layout_rows = tl.layout.len() / map.columns as usize;
    assert!(
        layout_rows > map.rows as usize,
        "tall doc must grow the canvas beyond the viewport"
    );
    assert_eq!(
        tl.rendered_rows as usize, layout_rows,
        "canvas must grow to exactly the content height (no screen cap)"
    );
}

/// Replace the children of `id="target"` with a single text node (the DOM
/// shape `el.textContent = v` leaves behind).
fn set_text_by_id(arena: &mut NodeArena, target: &str, text: &str) {
    let node = find_id(arena, target).expect("target node");
    let old = std::mem::take(&mut arena[node].children);
    for c in old {
        if let Some(n) = arena.get_mut(c) {
            n.parent = None;
        }
    }
    let text_id = arena.insert(Node::text(text));
    arena[text_id].parent = Some(node);
    arena[node].children.push(text_id);
}

#[test]
fn incremental_repaint_matches_full_repaint_after_value_changes() {
    // Golden test for the incremental path: after every simulated refresh
    // cycle (sysmon-style `textContent` updates), the incrementally repainted
    // canvas must be pixel-identical to a full re-draw. Any stale / missing
    // cell in the region repaint fails here, which would otherwise show up as
    // flickering or "jumping" digits on screen.
    let html = concat!(
        "<div>",
        "<p id=\"now\">now: 14:03:22 · session 00:05</p>",
        "<p id=\"cpu\">n/a</p>",
        "<p id=\"mem\">n/a</p>",
        "<p id=\"proc\">processes: n/a</p>",
        "</div>"
    );
    let (root_id, arena0) = setup(html);
    let map = term_map();
    let mut full = TermLayout::new(&map, 1.0);
    let mut incr = TermLayout::new(&map, 1.0);
    full.draw(&arena0, root_id, &map);
    incr.draw(&arena0, root_id, &map);
    let _ = full.render_diff_to_bytes();
    let _ = incr.render_diff_to_bytes();

    // Text lengths change across cycles (short→long, long→short) so wrapping
    // is exercised as well as in-place rewrites. The `cpu` cycles use the
    // sysmon progress-bar glyphs (U+2588 / U+2591) to prove the incremental
    // path stays pixel-identical for them too.
    let cycles: &[(&str, &str)] = &[
        ("now", "now: 14:03:23 · session 00:05"),
        ("cpu", "[████████░░░░░░░░] 45%"),
        ("mem", "8.1 / 15.9"),
        ("now", "now: 14:03:24 · session 00:06"),
        ("cpu", "[████████████░░░░] 98%"),
        ("proc", "processes: 1234"),
        ("cpu", "[██░░░░░░░░░░░░░░] 12%"),
        ("now", "now: 14:03:25 · session 00:07"),
        ("mem", "7.2 / 15.9"),
    ];
    let mut arena = arena0.clone();
    for &(id, text) in cycles {
        let mut next = arena.clone();
        set_text_by_id(&mut next, id, text);
        arena = next;

        // Full path: reset + layout + paint everything.
        full.draw(&arena, root_id, &map);
        let _ = full.render_diff_to_bytes();

        // Incremental path: layout only, repaint just the touched box rect.
        let boxes = incr.layout_only(&arena, root_id, &map);
        let node = find_id(&arena, id).expect("dirty node");
        let rect = find_box(&boxes, node).expect("dirty box").rect;
        incr.repaint_region(&arena, &boxes, rect);
        let _ = incr.render_diff_to_bytes();

        assert!(
            same_canvas(&full.layout, &incr.layout),
            "incremental canvas diverged from full after '{id}' = '{text}'"
        );
    }
}

/// Compare two canvases cell-by-cell (word / fg / bg / style).
fn same_canvas(a: &[htui::tui::graph_api::Block], b: &[htui::tui::graph_api::Block]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(x, y)| {
            x.word == y.word
                && x.fcolor == y.fcolor
                && x.bcolor == y.bcolor
                && x.style == y.style
        })
}

#[test]
fn viewport_resize_recreates_canvas_and_recovers() {
    // The render thread's resize path replaces the `TermLayout` (its
    // dimensions are fixed at construction) and re-renders from scratch —
    // this mirrors that: draw at one size, allocate a fresh canvas at a
    // different size, and verify the incremental renderer re-emits the full
    // viewport instead of silently keeping stale pixels.
    let (root_id, arena) = setup("<div>hello resize</div>");

    let small = TermMap {
        columns: 40,
        rows: 10,
    };
    let mut tl = TermLayout::new(&small, 1.0);
    tl.draw(&arena, root_id, &small);
    let first_bytes = tl.render_diff_to_bytes();
    let first = String::from_utf8_lossy(&first_bytes);
    assert!(
        first.contains("hello") && first.contains("resize"),
        "first frame must carry the document text"
    );

    // Grow: a fresh canvas has an empty `last_frame` snapshot, so the first
    // diff must clear the screen and emit every content cell again.
    let big = TermMap {
        columns: 100,
        rows: 30,
    };
    let mut grown = TermLayout::new(&big, 1.0);
    grown.draw(&arena, root_id, &big);
    let diff = grown.render_diff_to_bytes();
    assert!(!diff.is_empty(), "resize must re-emit the whole viewport");
    let text = String::from_utf8_lossy(&diff);
    assert!(
        text.contains("hello") && text.contains("resize"),
        "resize frame must carry the document text"
    );
    assert!(
        grown.render_diff_to_bytes().is_empty(),
        "stable view after resize must emit nothing"
    );

    // A stale scroll position beyond the new viewport must clamp, not panic.
    grown.scroll_y = 9999;
    grown.clamp_scroll();
    assert_eq!(grown.scroll_y, grown.max_scroll());
    let _ = grown.render_diff_to_bytes();

    // Shrink: fresh canvas again, still renders.
    let tiny = TermMap {
        columns: 20,
        rows: 5,
    };
    let mut shrunk = TermLayout::new(&tiny, 1.0);
    shrunk.draw(&arena, root_id, &tiny);
    assert!(
        !shrunk.render_diff_to_bytes().is_empty(),
        "shrunken viewport must re-emit content"
    );
}

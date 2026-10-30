use std::collections::HashMap;

use htui::{
    html::{css_parse, html_parse},
    layout::{
        BoxConstraint, Cursor, FormattingContext, GeneratedBoxKind, LayoutBox,
        inner_formatting_context::flow::{layout_tree, layout_tree_with_scroll_offsets},
        layout::TermMap,
    },
    tdom::node::{Node, NodeArena, NodeId, NodeType},
    tui::graph_api::TermLayout,
};

fn setup(html: &str, css: &str) -> (NodeId, NodeArena) {
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
    (root_id, arena)
}

#[test]
fn overflow_y_creates_a_native_scrollbar_and_translates_content() {
    let html = "<div id='list'>\
                <p>one</p><p>two</p><p>three</p>\
                <p>four</p><p>five</p><p>six</p>\
                </div>";
    let (root_id, arena) = setup(html, "#list { height: 80; overflow-y: auto; }");
    let list_id = arena
        .iter()
        .find(|(_, node)| node.get_id().map(String::as_str) == Some("list"))
        .map(|(id, _)| id)
        .expect("list exists");
    let map = term_map();
    let offsets = [(list_id, 2u32)].into_iter().collect::<HashMap<_, _>>();
    let mut cursor = Cursor::new();
    let boxes = layout_tree_with_scroll_offsets(
        &arena,
        root_id,
        BoxConstraint::root(&map),
        &mut cursor,
        map.columns as usize,
        map.rows as usize,
        &offsets,
        &std::collections::HashMap::new(),
    );
    let list = find_layout(&boxes, &arena, |node_type| {
        matches!(node_type, NodeType::Div)
    })
    .expect("scroll container exists");
    let scroll = list.paint_style.scrollbar.expect("active auto scrollbar");
    assert_eq!(scroll.scroll_y, 2);
    assert_eq!(scroll.max_scroll, 3);
    assert_eq!(scroll.track.height, 10);
    assert_eq!(scroll.viewport_height, 10);
    assert_eq!(
        scroll.content_height,
        scroll.max_scroll + scroll.viewport_height
    );
    assert_eq!(list.children.len(), 6);
    let first = &list.children[0];
    let second = &list.children[1];
    assert_eq!(second.rect.y, first.rect.y + 1);
    assert!(
        scroll
            .track
            .contains(scroll.thumb.x as u32, scroll.thumb.y as u32)
    );
}

#[test]
fn scrollbar_chrome_follows_visibility_while_track_geometry_stays_hittable() {
    let html = "<div id='list'>\
                <p>one</p><p>two</p><p>three</p>\
                <p>four</p><p>five</p><p>six</p>\
                </div>";
    let (root_id, arena) = setup(html, "#list { height: 10; overflow-y: auto; }");
    let map = term_map();
    let mut terminal = TermLayout::new(&map, 1.0);
    let boxes = terminal.draw(&arena, root_id, &map);
    let list = find_layout(&boxes, &arena, |node_type| {
        matches!(node_type, NodeType::Div)
    })
    .expect("scroll container exists");
    let track = list
        .paint_style
        .scrollbar
        .expect("overflowing container has a scrollbar")
        .track;

    terminal.scrollbars_visible.insert(list.node_id, false);
    terminal.draw(&arena, root_id, &map);
    let hidden = scrollbar_glyph_count(&terminal, &track);
    assert_eq!(hidden, 0, "idle scrollbar chrome must stay hidden");

    terminal.scrollbars_visible.insert(list.node_id, true);
    terminal.draw(&arena, root_id, &map);
    let visible = scrollbar_glyph_count(&terminal, &track);
    assert!(visible >= track.height.min(1));
}

fn scrollbar_glyph_count(terminal: &TermLayout, track: &htui::layout::Rect) -> usize {
    let width = terminal.columns as usize;
    (track.y..track.y + track.height)
        .map(|y| y * width + track.x)
        .filter_map(|index| terminal.layout.get(index))
        .filter(|cell| cell.word == "\u{2502}" || cell.word == "\u{2588}")
        .count()
}

#[test]
fn overflow_y_works_on_a_fixed_flex_item() {
    let html = "<section><div id='list'>\
                <p>one</p><p>two</p><p>three</p>\
                <p>four</p><p>five</p><p>six</p>\
                </div></section>";
    let (root_id, arena) = setup(
        html,
        "section { display: flex; flex-direction: column; } \
         #list { height: 24; overflow-y: auto; }",
    );
    let map = term_map();
    let mut cursor = Cursor::new();
    let boxes = layout_tree(
        &arena,
        root_id,
        BoxConstraint::root(&map),
        &mut cursor,
        map.columns as usize,
        map.rows as usize,
    );
    let list = find_layout(&boxes, &arena, |node_type| {
        matches!(node_type, NodeType::Div)
    })
    .expect("scrollable flex item exists");
    let scroll = list
        .paint_style
        .scrollbar
        .expect("flex item keeps its scroll container");
    assert!(scroll.max_scroll > 0);
    assert_eq!(list.children.len(), 6);
}

#[test]
fn clipping_flex_shell_does_not_grow_the_document_scroll_height() {
    let html = "<body><div>one</div><div>two</div><div>three</div>\
                <div>four</div><div>five</div><div>six</div></body>";
    let (root_id, arena) = setup(
        html,
        "body { height: 8; display: flex; flex-direction: column; overflow: hidden; }",
    );
    let map = term_map();
    let mut terminal = TermLayout::new(&map, 1.0);
    terminal.draw(&arena, root_id, &map);
    assert_eq!(
        terminal.max_scroll(),
        0,
        "a clipping viewport-fit shell must not create document scroll"
    );
}

/// `NodeId` of the first node matching `predicate`, in document order.
fn find_node(arena: &NodeArena, predicate: impl Fn(&NodeType) -> bool + Copy) -> NodeId {
    arena
        .iter()
        .find(|(_, n)| predicate(&n.node_type))
        .map(|(id, _)| id)
        .expect("node exists in document")
}

/// Classify one node through the shallow generation entry point.
fn classify(arena: &NodeArena, id: NodeId) -> htui::layout::GeneratedBox {
    htui::layout::generate_node(arena, id).expect("node should generate a box")
}

fn find_layout<'a>(
    boxes: &'a [LayoutBox],
    arena: &NodeArena,
    predicate: impl Fn(&NodeType) -> bool + Copy,
) -> Option<&'a LayoutBox> {
    boxes.iter().find_map(|layout_box| {
        if predicate(&arena[layout_box.node_id].node_type) {
            Some(layout_box)
        } else {
            find_layout(&layout_box.children, arena, predicate)
        }
    })
}

fn term_map() -> TermMap {
    TermMap {
        columns: 80,
        rows: 24,
    }
}

#[test]
fn overflow_x_creates_a_bottom_scrollbar_and_translates_content() {
    // A fixed-width `overflow-x: auto` container holding one nowrap child
    // far wider than the viewport: layout must expose a bottom one-row
    // track with a proportional thumb, report the measured content width,
    // and translate the subtree by the horizontal offset.
    let (root_id, arena) = setup(
        r#"<div id="h"><div id="wide">AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA</div></div>"#,
        "#h { width: 30ch; height: 3ch; overflow-x: auto; }\n#h div { white-space: nowrap; }",
    );
    let map = term_map();
    let mut cursor = Cursor::new();
    let boxes = layout_tree(
        &arena,
        root_id,
        BoxConstraint::root(&map),
        &mut cursor,
        map.columns as usize,
        map.rows as usize,
    );
    let container =
        find_layout(&boxes, &arena, |t| matches!(t, NodeType::Div)).expect("container exists");
    let scroll = container
        .paint_style
        .scrollbar
        .expect("horizontal overflow activates the scrollbar");
    assert!(
        scroll.max_scroll_x > 0,
        "content wider than 30 columns: content_width={}",
        scroll.content_width
    );
    assert_eq!(scroll.viewport_width, 30, "content-box width");
    assert!(scroll.content_width > 30);
    assert_eq!(scroll.h_track.height, 1, "one-row bottom track");
    assert_eq!(scroll.h_track.width, 30);
    assert_eq!(
        scroll.h_track.y,
        container.rect.y + container.rect.height - 1
    );
    assert!(
        scroll.h_thumb.width >= 1 && scroll.h_thumb.width <= 30,
        "thumb inside the track"
    );
    // Un-scrolled: content sits at its natural offset.
    let wide = container.children.first().expect("wide child in the tree");
    assert_eq!(wide.rect.x, container.rect.x);

    // Apply a horizontal offset: the child subtree shifts left, the thumb
    // slides right.
    let offsets = std::collections::HashMap::new();
    let x_offsets = [(container.node_id, 4u32)].into_iter().collect();
    let mut cursor = Cursor::new();
    let scrolled = layout_tree_with_scroll_offsets(
        &arena,
        root_id,
        BoxConstraint::root(&map),
        &mut cursor,
        map.columns as usize,
        map.rows as usize,
        &offsets,
        &x_offsets,
    );
    let container =
        find_layout(&scrolled, &arena, |t| matches!(t, NodeType::Div)).expect("container exists");
    let scroll = container
        .paint_style
        .scrollbar
        .expect("scrollbar survives scrolling");
    assert_eq!(scroll.scroll_x, 4, "offset applied from the x table");
    let wide = container.children.first().expect("wide child in the tree");
    assert_eq!(
        wide.rect.x,
        container.rect.x.saturating_sub(4),
        "content translated left by the offset"
    );
    assert!(
        scroll.h_thumb.x > scroll.h_track.x,
        "thumb slides toward the track end (scroll_x={} max_scroll_x={} content_width={} thumb_x={} track_x={})",
        scroll.scroll_x,
        scroll.max_scroll_x,
        scroll.content_width,
        scroll.h_thumb.x,
        scroll.h_track.x
    );
}

#[test]
fn scrollbar_region_maps_pointer_columns_to_offsets() {
    // Pure math over the input-facing region: `offset_at_x` maps a pointer
    // column onto the content offset, honouring the grab offset.
    let region = htui::events::ScrollbarRegion {
        node_id: Default::default(),
        track: htui::layout::Rect::new(0, 0, 0, 0),
        thumb: htui::layout::Rect::new(0, 0, 0, 0),
        scroll_y: 0,
        max_scroll: 0,
        content_height: 0,
        viewport_height: 0,
        scroll_x: 0,
        max_scroll_x: 40,
        content_width: 50,
        viewport_width: 10,
        h_track: htui::layout::Rect::new(0, 3, 10, 1),
        h_thumb: htui::layout::Rect::new(0, 3, 2, 1),
    };
    assert_eq!(region.offset_at_x(0, 0), 0, "track start = 0");
    assert_eq!(region.offset_at_x(10, 0), 40, "track end = max");
    let mid = region.offset_at_x(5, 0);
    assert!(
        (15..=25).contains(&mid),
        "mid track ≈ mid offset, got {mid}"
    );
    // Grab offset shifts the pointer→offset mapping.
    assert_eq!(region.offset_at_x(2, 2), 0, "grabbed thumb start stays 0");
}

#[test]
fn box_generation_classifies_flow_flex_table_and_replaced_boxes() {
    let html = "<div><span>inline</span> text</div>\
                <section><p>item</p></section>\
                <table><tr><td>cell</td></tr></table>\
                <img src='missing.png' width='8' height='8'>";
    let (_root_id, arena) = setup(html, "section { display: flex; }");

    // Generation is a shallow per-node classifier: each element classifies
    // into its (kind, outer display, formatting context) triple exactly once.
    let div = classify(&arena, find_node(&arena, |nt| matches!(nt, NodeType::Div)));
    assert_eq!(div.inner_formatting_context, FormattingContext::Flow);

    let section = classify(
        &arena,
        find_node(&arena, |nt| matches!(nt, NodeType::Section)),
    );
    assert_eq!(section.inner_formatting_context, FormattingContext::Flex);

    let table = classify(
        &arena,
        find_node(&arena, |nt| matches!(nt, NodeType::Table)),
    );
    assert_eq!(table.inner_formatting_context, FormattingContext::Table);

    let image = classify(&arena, find_node(&arena, |nt| matches!(nt, NodeType::Img)));
    assert_eq!(image.kind, GeneratedBoxKind::Replaced);
}

#[test]
fn geometry_pass_dispatches_flow_flex_table_and_replaced_boxes() {
    let html = "<div>flow</div>\
                <section><p>A</p><p>B</p></section>\
                <table><tr><td>T</td></tr></table>\
                <img src='missing.png' width='8' height='8'>";
    let (root_id, arena) = setup(html, "section { display: flex; }");
    let map = term_map();
    let mut cursor = Cursor::new();
    let boxes = layout_tree(
        &arena,
        root_id,
        BoxConstraint::root(&map),
        &mut cursor,
        map.columns as usize,
        map.rows as usize,
    );

    let flex = find_layout(&boxes, &arena, |node_type| {
        matches!(node_type, NodeType::Section)
    })
    .expect("flex geometry should be produced");
    assert_eq!(flex.inner_formatting_context, FormattingContext::Flex);
    assert_eq!(flex.children.len(), 2);

    let table = find_layout(&boxes, &arena, |node_type| {
        matches!(node_type, NodeType::Table)
    })
    .expect("table geometry should be produced");
    assert!(!table.children.is_empty());

    let image = find_layout(&boxes, &arena, |node_type| {
        matches!(node_type, NodeType::Img)
    })
    .expect("replaced geometry should be produced");
    assert!(matches!(image.kind, htui::layout::BoxKind::Replaced(_)));
}

#[test]
fn unified_pipeline_renders_mixed_document_end_to_end() {
    let html = "<div>Flow</div>\
                <section><p>FlexA</p><p>FlexB</p></section>\
                <table><tr><td>TableCell</td></tr></table>";
    let (root_id, arena) = setup(html, "section { display: flex; }");
    let map = term_map();
    let mut terminal_layout = TermLayout::new(&map, 1.0);
    terminal_layout.draw(&arena, root_id, &map);

    let canvas: String = terminal_layout
        .layout
        .iter()
        .map(|cell| cell.word.as_str())
        .collect();
    assert!(canvas.contains("Flow"));
    assert!(canvas.contains("FlexA"));
    assert!(canvas.contains("FlexB"));
    assert!(canvas.contains("TableCell"));
}

#[test]
fn sibling_top_level_boxes_paint_on_distinct_rows() {
    // A root with two rendered children (no `<html>` wrapper) yields two
    // top-level boxes that must paint their own rows sequentially across a
    // multi-box forest.
    let mut arena = NodeArena::default();
    let root = arena.insert(Node::from(NodeType::Document));
    let div1 = arena.insert(Node::from(NodeType::Div));
    let text1 = arena.insert(Node::text("first band"));
    let div2 = arena.insert(Node::from(NodeType::Div));
    let text2 = arena.insert(Node::text("second band"));
    Node::append_child(&mut arena, div1, text1);
    Node::append_child(&mut arena, div2, text2);
    Node::append_child(&mut arena, root, div1);
    Node::append_child(&mut arena, root, div2);

    let map = term_map();
    let mut terminal_layout = TermLayout::new(&map, 1.0);
    terminal_layout.draw(&arena, root, &map);

    let canvas: String = terminal_layout
        .layout
        .iter()
        .map(|cell| cell.word.as_str())
        .collect();
    assert!(canvas.contains("first band"), "first band should render");
    assert!(canvas.contains("second band"), "second band should render");

    // The two boxes land on distinct rows: the second box must not paint
    // over the first one's cells.
    let cols = map.columns as usize;
    let rows = terminal_layout.layout.len() / cols;
    let word_rows: Vec<String> = (0..rows)
        .map(|row| {
            terminal_layout.layout[row * cols..(row + 1) * cols]
                .iter()
                .map(|cell| cell.word.as_str())
                .collect()
        })
        .collect();
    let first_row = word_rows
        .iter()
        .position(|line| line.contains("first band"))
        .expect("first band row");
    let second_row = word_rows
        .iter()
        .position(|line| line.contains("second band"))
        .expect("second band row");
    assert_ne!(
        first_row, second_row,
        "top-level boxes must paint on different rows"
    );
}

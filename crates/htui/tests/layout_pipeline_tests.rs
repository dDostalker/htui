use htui::{
    html::{css_parse, html_parse},
    layout::{
        BoxConstraint, Cursor, FormattingContext, GeneratedBoxKind, LayoutBox,
        inner_formatting_context::flow::layout_tree, layout::TermMap,
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
    // top-level boxes that must paint their own rows sequentially — a
    // regression guard for the paint pass over multi-box forests.
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

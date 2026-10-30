//! CSS Grid layout tests for the terminal-grid v1 subset.
//!
//! These tests intentionally assert visible terminal cells rather than
//! private track calculations: placement, span, gaps, dense back-fill, and
//! item alignment are all part of the public rendering contract.

use htui::html::{css_parse, html_parse};
use htui::layout::layout::TermMap;
use htui::tui::graph_api::TermLayout;

const COLS: u16 = 40;
const ROWS: u16 = 16;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

fn setup(html: &str, css: &str) -> (htui::tdom::node::NodeId, htui::tdom::node::NodeArena) {
    let mut p = html_parse::Parser::new(html);
    let (root_id, mut arena, embedded) = p.parse();
    let combined = if css.is_empty() {
        embedded
    } else if embedded.is_empty() {
        css.to_string()
    } else {
        format!("{embedded}\n{css}")
    };
    let mut cp = css_parse::Parser::new(&combined);
    css_parse::css_into_node(&mut arena, root_id, &cp.parse());
    (root_id, arena)
}

fn render_rows(html: &str, css: &str) -> Vec<String> {
    let (root_id, arena) = setup(html, css);
    let term_map = TermMap {
        columns: COLS,
        rows: ROWS,
    };
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map);
    (0..term_map.rows)
        .map(|row| {
            let start = row as usize * COLS as usize;
            let end = start + COLS as usize;
            tl.layout[start..end]
                .iter()
                .map(|block| block.word.as_str())
                .collect()
        })
        .collect()
}

#[test]
fn grid_auto_places_row_items_in_explicit_columns() {
    init();
    let rows = render_rows(
        r#"<div class="g"><div>A</div><div>B</div><div>C</div></div>"#,
        ".g { display: grid; grid-template-columns: 3ch 3ch; }",
    );
    assert!(
        rows[0].starts_with("A  B"),
        "two columns are filled left-to-right: {:?}",
        rows[0]
    );
    assert!(
        rows[1].starts_with("C"),
        "the third item wraps to a new implicit row: {:?}",
        rows[1]
    );
}

#[test]
fn grid_fr_tracks_and_gap_distribute_the_container() {
    init();
    let rows = render_rows(
        r#"<div class="g"><div>A</div><div>B</div></div>"#,
        ".g { display: grid; width: 7ch; grid-template-columns: 1fr 1fr; column-gap: 1ch; }",
    );
    assert_eq!(
        rows[0].chars().next(),
        Some('A'),
        "first fr track starts at the container edge: {:?}",
        rows[0]
    );
    assert_eq!(
        rows[0].chars().nth(4),
        Some('B'),
        "second fr track starts after 3 cells and a one-cell gap: {:?}",
        rows[0]
    );
}

#[test]
fn grid_explicit_line_placement_and_span() {
    init();
    let rows = render_rows(
        r#"<div class="g"><div class="a">A</div><div class="b">B<br>D</div></div>"#,
        r#"
        .g { display: grid; grid-template-columns: 2ch 2ch 2ch; grid-template-rows: 1ch 1ch; }
        .a { grid-column: 1 / span 2; }
        .b { grid-column: 3; grid-row: 1 / 3; }
        "#,
    );
    assert_eq!(
        rows[0].find('A'),
        Some(0),
        "A starts in track one: {:?}",
        rows[0]
    );
    assert_eq!(
        rows[0].find('B'),
        Some(4),
        "B starts in track three: {:?}",
        rows[0]
    );
    assert_eq!(
        rows[1].find('D'),
        Some(4),
        "B's two-row area reaches row two: {:?}",
        rows[1]
    );
    assert!(
        !rows[1].contains('A'),
        "A occupies only the first row: {:?}",
        rows[1]
    );
}

#[test]
fn grid_dense_placement_fills_an_earlier_hole() {
    init();
    let rows = render_rows(
        r#"
        <div class="g">
          <div class="a">A</div>
          <div class="b">B<br>D</div>
          <div class="c">C</div>
        </div>
        "#,
        r#"
        .g { display: grid; grid-auto-flow: row dense; grid-template-columns: 2ch 2ch; }
        .a { grid-column: 2; }
        .b { grid-column: 2; }
        .c { grid-column: span 1; }
        "#,
    );
    // B explicitly takes column two on row one. Dense allows C to back-fill
    // the hole in column one instead of starting a third row.
    assert!(
        rows[0].starts_with('C'),
        "dense placement fills column one with C: {:?}",
        rows[0]
    );
    assert_eq!(
        rows[0].chars().nth(2),
        Some('A'),
        "explicit A remains in column two: {:?}",
        rows[0]
    );
    assert!(
        rows[1].chars().nth(2) == Some('B'),
        "explicit B stays on row two/column two: {:?}",
        rows[1]
    );
}

#[test]
fn grid_align_self_positions_a_narrow_item() {
    init();
    let rows = render_rows(
        r#"<div class="g"><div class="a">A</div></div>"#,
        r#"
        .g { display: grid; grid-template-columns: 6ch; grid-template-rows: 3ch; }
        .a { justify-self: center; align-self: end; }
        "#,
    );
    let row2 = &rows[2];
    let pos = row2.find('A').expect("grid item on bottom row");
    assert_eq!(
        pos, 2,
        "a one-cell item centered in a six-cell track uses the integer-cell center: {row2:?}"
    );
}

#[test]
fn grid_items_support_order() {
    init();
    let rows = render_rows(
        r#"<div class="g"><div class="a">A</div><div class="b">B<br>D</div></div>"#,
        ".g { display: grid; grid-template-columns: 2ch 2ch; } .a { order: 2; } .b { order: 1; }",
    );
    assert!(
        rows[0].starts_with("B A"),
        "grid placement follows the order-modified document order: {:?}",
        rows[0]
    );
}

#[test]
fn grid_item_text_does_not_duplicate_as_container_run() {
    init();
    let rows = render_rows(
        r#"<div class="outer">X<div class="g"><div class="a">A</div></div>Y</div>"#,
        ".g { display: grid; grid-template-columns: 2ch; }",
    );
    let all = rows.join("\n");
    assert_eq!(
        all.matches('A').count(),
        1,
        "grid item content renders exactly once: {all:?}"
    );
}

use crossterm::style::Color;
use htui::html::css_parse;
use htui::html::html_parse;
use htui::layout::layout::TermMap;
use htui::tdom::node::{NodeArena, NodeId, NodeType};
use htui::tui::graph_api::TermLayout;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
}

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

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn mock_term_map() -> TermMap {
    TermMap {
        columns: COLS,
        rows: ROWS,
    }
}

fn cell_containing(tl: &TermLayout, needle: &str) -> Option<(usize, usize, usize)> {
    for (idx, block) in tl.layout.iter().enumerate() {
        if block.word == needle {
            let cols = tl.columns as usize;
            return Some((idx, idx / cols, idx % cols));
        }
    }
    None
}

#[test]
fn test_parse_tag_map_table() {
    assert_eq!(
        html_parse::tag_name_to_node_type("table"),
        Some(NodeType::Table)
    );
    assert_eq!(html_parse::tag_name_to_node_type("tr"), Some(NodeType::Tr));
    assert_eq!(html_parse::tag_name_to_node_type("td"), Some(NodeType::Td));
    assert_eq!(html_parse::tag_name_to_node_type("th"), Some(NodeType::Th));
    assert_eq!(
        html_parse::tag_name_to_node_type("thead"),
        Some(NodeType::Thead)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("tbody"),
        Some(NodeType::Tbody)
    );
    assert_eq!(
        html_parse::tag_name_to_node_type("tfoot"),
        Some(NodeType::Tfoot)
    );
}

#[test]
fn test_parse_tag_map_uppercase_table() {
    assert_eq!(
        html_parse::tag_name_to_node_type("TABLE"),
        Some(NodeType::Table)
    );
    assert_eq!(html_parse::tag_name_to_node_type("TR"), Some(NodeType::Tr));
    assert_eq!(html_parse::tag_name_to_node_type("TD"), Some(NodeType::Td));
    assert_eq!(html_parse::tag_name_to_node_type("TH"), Some(NodeType::Th));
}

#[test]
fn test_table_renders_all_cell_text() {
    init();
    let html = "<table><tr><td>Apple</td><td>Banana</td></tr><tr><td>Cherry</td><td>Date</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Apple"), "missing 'Apple' cell");
    assert!(canvas.contains("Banana"), "missing 'Banana' cell");
    assert!(canvas.contains("Cherry"), "missing 'Cherry' cell");
    assert!(canvas.contains("Date"), "missing 'Date' cell");
}

#[test]
fn test_table_renders_th_header_text() {
    init();
    let html = "<table><tr><th>Name</th><th>Role</th></tr><tr><td>Alice</td><td>Engineer</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Name"), "header 'Name' missing");
    assert!(canvas.contains("Role"), "header 'Role' missing");
    assert!(canvas.contains("Alice"), "data 'Alice' missing");
    assert!(canvas.contains("Engineer"), "data 'Engineer' missing");
}

#[test]
fn test_table_th_text_marks_bold() {
    // <th> should pick up the CSS default text-decoration? No, we don't
    // add <th> defaults — but with explicit CSS we should still see bold.
    init();
    let html = "<table><tr><th>H1</th><th>H2</th></tr><tr><td>D1</td><td>D2</td></tr></table>";
    let css = "th { font-weight: bold; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let bold_letters: String = tl
        .layout
        .iter()
        .filter(|b| b.style.contains(htui::tui::graph_api::FontStyle::BOLD))
        .map(|b| b.word.as_str())
        .collect();
    assert!(
        bold_letters.contains('H'),
        "<th> header letters should be bold via CSS rule; got {bold_letters:?}"
    );
}

#[test]
fn test_table_cells_align_in_columns() {
    // Two rows of two cells should align: each row's cell 1 sits on the same
    // starting column, and cell 2's column is shared too.
    init();
    let html = "<table><tr><td>AB</td><td>CD</td></tr><tr><td>EF</td><td>GH</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let cols = tl.columns as usize;
    let a_idx = cell_containing(&tl, "A").expect("missing 'A'").0;
    let c_idx = cell_containing(&tl, "C").expect("missing 'C'").0;
    let e_idx = cell_containing(&tl, "E").expect("missing 'E'").0;
    let g_idx = cell_containing(&tl, "G").expect("missing 'G'").0;

    let a_col = a_idx % cols;
    let c_col = c_idx % cols;
    let e_col = e_idx % cols;
    let g_col = g_idx % cols;

    assert_eq!(
        a_col, e_col,
        "row 1 cell 1 and row 2 cell 1 must share a column"
    );
    assert_eq!(
        c_col, g_col,
        "row 1 cell 2 and row 2 cell 2 must share a column"
    );
    assert_ne!(
        a_col, c_col,
        "cell 1 and cell 2 must be in different columns"
    );
}

#[test]
fn test_table_cells_on_consecutive_rows() {
    init();
    let html = "<table><tr><td>Row1</td></tr><tr><td>Row2</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let (_, row1_row, _) = cell_containing(&tl, "R").expect("missing 'R' of row1");
    let (_, row2_row, _) = tl
        .layout
        .iter()
        .enumerate()
        .filter_map(|(idx, b)| {
            if idx / (tl.columns as usize) > row1_row && b.word == "R" {
                let row = idx / tl.columns as usize;
                let col = idx % tl.columns as usize;
                Some((idx, row, col))
            } else {
                None
            }
        })
        .next()
        .expect("missing 'R' of row2");

    assert!(
        row2_row > row1_row,
        "row 2 (row={row2_row}) must be below row 1 (row={row1_row})"
    );
}

#[test]
fn test_table_with_thead_section() {
    init();
    let html = "<table><thead><tr><th>Header</th></tr></thead><tbody><tr><td>Body</td></tr></tbody></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Header"), "thead content missing");
    assert!(canvas.contains("Body"), "tbody content missing");
}

#[test]
fn test_table_with_tfoot_section() {
    init();
    let html = "<table><thead><tr><th>Item</th></tr></thead><tbody><tr><td>Data</td></tr></tbody><tfoot><tr><td>Foot</td></tr></tfoot></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Item"), "thead content missing");
    assert!(canvas.contains("Data"), "tbody content missing");
    assert!(canvas.contains("Foot"), "tfoot content missing");
}

#[test]
fn test_table_css_border_draws_box_glyphs() {
    init();
    let html = "<table><tr><td>A</td><td>B</td></tr></table>";
    let css = "table, td { border: 1px solid white; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(
        canvas.contains('─') || canvas.contains('│'),
        "table border should draw at least one ─ or │ glyph"
    );
    assert!(
        canvas.contains('┌')
            || canvas.contains('┐')
            || canvas.contains('└')
            || canvas.contains('┘'),
        "table border should draw at least one corner glyph"
    );
}

#[test]
fn test_table_cell_color_applies() {
    init();
    let html = "<table><tr><td>Red</td></tr></table>";
    let css = "td { color: red; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let has_red = tl.layout.iter().any(|b| b.fcolor == Some(Color::Red));
    assert!(has_red, "td should pick up `color: red` via CSS selector");
}

#[test]
fn test_table_cell_bg_color_applies() {
    init();
    let html = "<table><tr><td>Yellow cell</td></tr></table>";
    let css = "td { background-color: yellow; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let yellow_count = tl
        .layout
        .iter()
        .filter(|b| b.bcolor == Some(Color::Yellow))
        .count();
    assert!(
        yellow_count > 0,
        "td background-color: yellow should paint at least one cell"
    );
}

#[test]
fn test_table_nth_child_even_stripes_td_bg() {
    // Mirrors the `table/index.html` visual test: `tr:nth-child(even) td` and
    // `tr:nth-child(odd) td` must colour the even / odd rows' cells
    // distinctly. Before the `:nth-child()` selector was implemented, both
    // rules silently no-op'd so all cells kept the same background.
    init();
    let html = "<table>\
        <tr><td>R1</td></tr>\
        <tr><td>R2</td></tr>\
        <tr><td>R3</td></tr>\
        <tr><td>R4</td></tr>\
        </table>";
    let css = "tr:nth-child(even) td { background-color: darkgrey; } \
               tr:nth-child(odd) td { background-color: black; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let even_cells = tl
        .layout
        .iter()
        .filter(|b| b.bcolor == Some(Color::Grey))
        .count();
    let odd_cells = tl
        .layout
        .iter()
        .filter(|b| b.bcolor == Some(Color::Black))
        .count();
    assert!(
        even_cells > 0,
        "nth-child(even) must colour at least one cell darkgrey (got {even_cells})"
    );
    assert!(
        odd_cells > 0,
        "nth-child(odd) must colour at least one cell black (got {odd_cells})"
    );
}

#[test]
fn test_table_caption_silently_dropped() {
    // <caption> is parsed but not rendered — its text must not appear.
    init();
    let html = "<table><caption>Hidden</caption><tr><td>Visible</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Visible"), "td text should render");
    assert!(
        !canvas.contains("Hidden"),
        "caption text should not appear in render"
    );
}

#[test]
fn test_table_colgroup_silently_dropped() {
    // <colgroup> is parsed but not rendered — table should still render
    // its rows normally.
    init();
    let html = "<table><colgroup><col><col></colgroup><tr><td>A</td><td>B</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("A"), "cell A should render");
    assert!(canvas.contains("B"), "cell B should render");
}

#[test]
fn test_table_empty_renders_empty_box() {
    init();
    let (root_id, arena) = setup("<table></table>", "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();
    // No assertion needed; just verify draw doesn't fail.
}

#[test]
fn test_table_row_with_no_cells_keeps_row_geometry() {
    init();
    let html = "<table><tr></tr><tr><td>Below</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Below"), "second-row content must render");
}

#[test]
fn test_table_short_row_pads_with_empty_columns() {
    // A row with fewer cells than the column count should still align its
    // existing cells with the wider rows above.
    init();
    let html = "<table><tr><td>Long</td><td>Wide</td></tr><tr><td>Short</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("Long"), "wide row cell 1 missing");
    assert!(canvas.contains("Wide"), "wide row cell 2 missing");
    assert!(canvas.contains("Short"), "short row cell 1 missing");
}

#[test]
fn test_table_does_not_crash_on_text_between_rows() {
    // Whitespace / text nodes between rows are flattened by the HTML parser,
    // so the table should still render its rows.
    init();
    let html = "<table><tr><td>A</td></tr>  <tr><td>B</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(canvas.contains("A"), "first row missing");
    assert!(canvas.contains("B"), "second row missing");
}

#[test]
fn test_table_three_column_alignment() {
    init();
    let html = "<table><tr><td>A</td><td>B</td><td>C</td></tr><tr><td>D</td><td>E</td><td>F</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let cols = tl.columns as usize;
    let a_col = cell_containing(&tl, "A").unwrap().0 % cols;
    let b_col = cell_containing(&tl, "B").unwrap().0 % cols;
    let c_col = cell_containing(&tl, "C").unwrap().0 % cols;
    let d_col = cell_containing(&tl, "D").unwrap().0 % cols;
    let e_col = cell_containing(&tl, "E").unwrap().0 % cols;
    let f_col = cell_containing(&tl, "F").unwrap().0 % cols;

    assert_eq!(a_col, d_col, "col 1 aligned");
    assert_eq!(b_col, e_col, "col 2 aligned");
    assert_eq!(c_col, f_col, "col 3 aligned");
    assert!(
        a_col < b_col && b_col < c_col,
        "columns monotonically increase"
    );
}

#[test]
fn test_table_column_width_grows_to_widest_cell() {
    init();
    let html = "<table><tr><td>A</td><td>very wide cell content</td></tr></table>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let a_idx = cell_containing(&tl, "A").unwrap().0;
    let v_idx = cell_containing(&tl, "v").unwrap().0;
    // Cell 2 must start at least 2 cells to the right of cell 1 (cell 1
    // needs at least one cell of content width + padding).
    let cell_1_width = v_idx - a_idx;
    assert!(
        cell_1_width >= 3,
        "cell 1 should be at least 3 cells wide (CELL_PADDING + content), got {cell_1_width}"
    );
}

#[test]
fn test_table_text_align_center_in_cell() {
    init();
    let html = "<table><tr><td>X</td></tr></table>";
    let css = "td { text-align: center; }";
    let (root_id, arena) = setup(html, css);
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    // Find the column of the 'X' cell. With a wide column (3 minimum +
    // padding), X should be offset from the left edge of the column.
    let x_idx = cell_containing(&tl, "X").unwrap().0;
    let x_col = x_idx % (tl.columns as usize);
    assert!(
        x_col > 0,
        "X should be offset from the left edge (centered), got col {x_col}"
    );
}

#[test]
fn test_table_outside_table_falls_back_to_block() {
    // <tr>/<td> outside a table should still render via the fallback
    // block layout so their text is not silently dropped.
    init();
    let html = "<tr><td>stray</td></tr>";
    let (root_id, arena) = setup(html, "");
    let term_map = mock_term_map();
    let mut tl = TermLayout::new(&term_map, 1.0);
    tl.draw(&arena, root_id, &term_map).unwrap();

    let canvas: String = tl.layout.iter().map(|b| b.word.as_str()).collect();
    assert!(
        canvas.contains("stray"),
        "stray <td> text should still render via fallback block"
    );
}

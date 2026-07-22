use std::path::PathBuf;

use htui::tui::graph_api::Img;

#[test]
fn test_create_img() {
    let mut img = Img::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pic.png"),
        (320, 160),
    );
    img._create_img().unwrap();
    assert!(img.cloumns > 0, "image should have columns");
    assert!(img.rows > 0, "image should have rows");
    assert!(img.picture.is_some(), "image should have picture data");
}

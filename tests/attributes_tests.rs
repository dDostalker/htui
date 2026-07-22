use htui::tdom::attributes::Attr;

#[test]
fn test_form_attrs() {
    let a = Attr::parse_from_str("method=\"post\" action=\"/submit\"");
    assert_eq!(a.get_method().map(|s| s.as_str()), Some("post"));
    assert_eq!(a.get_action().map(|s| s.as_str()), Some("/submit"));
}

#[test]
fn test_form_novalidate() {
    let a = Attr::parse_from_str("novalidate");
    assert!(a.get_novalidate());
    let a = Attr::parse_from_str("");
    assert!(!a.get_novalidate());
}

#[test]
fn test_form_misc() {
    let a = Attr::parse_from_str("enctype=\"multipart/form-data\"");
    assert_eq!(
        a.get_enctype().map(|s| s.as_str()),
        Some("multipart/form-data")
    );
}

#[test]
fn test_select_attrs() {
    let a = Attr::parse_from_str("name=\"country\" size=\"5\"");
    assert_eq!(a.get_name().map(|s| s.as_str()), Some("country"));
    assert_eq!(a.get_size(), Some(5));
}

#[test]
fn test_option_attrs() {
    let a = Attr::parse_from_str("value=\"us\" selected");
    assert_eq!(a.get_value().map(|s| s.as_str()), Some("us"));
    assert!(a.get_selected());
}

#[test]
fn test_button_type() {
    let a = Attr::parse_from_str("type=\"submit\"");
    assert_eq!(a.get_button_type().map(|s| s.as_str()), Some("submit"));
}

#[test]
fn test_textarea_attrs() {
    let a = Attr::parse_from_str("rows=\"10\" cols=\"40\" wrap=\"soft\"");
    assert_eq!(a.get_rows(), Some(10));
    assert_eq!(a.get_cols(), Some(40));
    assert_eq!(a.get_wrap().map(|s| s.as_str()), Some("soft"));
}

#[test]
fn test_label_for() {
    let a = Attr::parse_from_str("for=\"email\"");
    assert_eq!(a.get_label_for().map(|s| s.as_str()), Some("email"));
}

#[test]
fn test_parse_empty() {
    let a = Attr::parse_from_str("");
    assert!(a.get("anything").is_none());
}

#[test]
fn test_parse_multi_attrs() {
    let a = Attr::parse_from_str("class=\"btn primary\" id=\"submit-btn\" disabled");
    assert_eq!(
        a.get_class(),
        Some(vec!["btn".to_string(), "primary".to_string()])
    );
    assert_eq!(a.get_id(), Some(&"submit-btn".to_string()));
    assert!(a.get_disabled());
}

#[test]
fn test_parse_malformed_no_value() {
    let a = Attr::parse_from_str("disabled checked");
    assert!(a.get_disabled());
}

#[test]
fn test_parse_malformed_no_equals() {
    let a = Attr::parse_from_str("readonly");
    assert!(a.get_readonly());
}

#[test]
fn test_parse_quoted_edge() {
    let a = Attr::parse_from_str("title=\"He said: hello\"");
    assert_eq!(a.get_title().map(|s| s.as_str()), Some("He said: hello"));
}

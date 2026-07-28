use super::cell::FontStyle;

/// Aggregate the CSS font/text style on a node into a [`FontStyle`] bitfield.
pub fn block_style(css_attr: &crate::html::css_parse::CssAttr) -> FontStyle {
    use crate::html::css_parse::{FontStyleKind, FontWeight, TextDecoration};
    let mut style = FontStyle::empty();
    if matches!(css_attr.get_font_weight(), FontWeight::Bold) {
        style |= FontStyle::BOLD;
    }
    if !matches!(css_attr.get_font_style_kind(), FontStyleKind::Normal) {
        style |= FontStyle::ITALIC;
    }
    match css_attr.get_text_decoration() {
        TextDecoration::Underline => style |= FontStyle::UNDERLINED,
        TextDecoration::LineThrough => style |= FontStyle::CROSSED_OUT,
        TextDecoration::UnderlineLineThrough => {
            style |= FontStyle::UNDERLINED | FontStyle::CROSSED_OUT;
        }
        TextDecoration::None => {}
    }
    style
}

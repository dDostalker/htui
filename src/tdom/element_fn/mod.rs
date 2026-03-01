use anyhow::Result;
mod a;
mod article;
mod aside;
mod button;
mod details;
mod div;
mod figcaption;
mod figure;
mod footer;
mod form;
mod header;
mod heading;
mod htmloption;
mod img;
mod input;
mod label;
mod li;
mod main;
mod nav;
mod ol;
mod p;
mod section;
mod select;
mod span;
mod summary;
mod textarea;
mod ul;
use crate::tdom::{
    attributes::Attr,
    element_fn::{
        a::a_element,
        article::article_element,
        aside::aside_element,
        button::button_element,
        details::details_element,
        div::div_element,
        figcaption::figcaption_element,
        figure::figure_element,
        footer::footer_element,
        form::form_element,
        header::header_element,
        heading::{h1_element, h2_element, h3_element, h4_element, h5_element, h6_element},
        htmloption::htmloption_element,
        img::img_element,
        input::input_element,
        label::label_element,
        li::li_element,
        main::main_element,
        nav::nav_element,
        ol::ol_element,
        p::p_element,
        section::section_element,
        select::select_element,
        span::span_element,
        summary::summary_element,
        textarea::textarea_element,
        ul::ul_element,
    },
    node::NodeType,
};

pub type ElementFn = fn(&str) -> Result<Option<(NodeType, Attr)>>;

pub static ELEMENT_FUNCTIONS: &[ElementFn] = &[
    a_element,
    article_element,
    aside_element,
    button_element,
    details_element,
    div_element,
    figcaption_element,
    figure_element,
    footer_element,
    form_element,
    header_element,
    h1_element,
    h2_element,
    h3_element,
    h4_element,
    h5_element,
    h6_element,
    img_element,
    input_element,
    label_element,
    li_element,
    main_element,
    nav_element,
    ol_element,
    htmloption_element,
    p_element,
    section_element,
    select_element,
    span_element,
    summary_element,
    textarea_element,
    ul_element,
];

// #[test]
// fn test_static_list() {
//     for func in ELEMENT_FUNCTIONS {
//         func("<div>");
//     }

// }

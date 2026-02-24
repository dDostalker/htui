use anyhow::Result;
mod div;
mod heading;
mod img;
mod p;
mod span;
use crate::tdom::{
    attributes::Attr,
    element_fn::{
        div::div_element,
        heading::{h1_element, h2_element, h3_element, h4_element, h5_element, h6_element},
        img::img_element,
        p::p_element,
        span::span_element,
    },
    node::NodeType,
};

pub type ElementFn = fn(&str) -> Result<Option<(NodeType, Attr)>>;

pub static ELEMENT_FUNCTIONS: &[ElementFn] = &[
    div_element,
    p_element,
    span_element,
    img_element,
    h1_element,
    h2_element,
    h3_element,
    h4_element,
    h5_element,
    h6_element,
];

// #[test]
// fn test_static_list() {
//     for func in ELEMENT_FUNCTIONS {
//         func("<div>");
//     }

// }

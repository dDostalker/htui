use anyhow::Result;
mod div;
mod p;
mod span;
mod img;
mod heading;
use crate::tdom::{
    element_fn::{
        div::div_element, 
        p::p_element, 
        span::span_element,
        img::img_element,
        heading::{h1_element, h2_element, h3_element, h4_element, h5_element, h6_element},
    },
    node::NodeType,
    attributes::Attr
};

pub type ElementFn = fn(&str) -> Result<Option<(NodeType,Attr)>>;

pub static ELEMENT_FUNCTIONS: &[ElementFn] = &[
    div_element, 
    p_element, 
    span_element,
    img_element,
    h1_element,  // 添加
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

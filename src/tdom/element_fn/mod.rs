use anyhow::Result;
mod div;
mod p;
mod span;
use crate::tdom::{
    element_fn::{div::div_element, p::p_element, span::span_element},
    node::NodeType,
    attributes::Attr
};

pub type ElementFn = fn(&str) -> Result<Option<(NodeType,Attr)>>;

pub static ELEMENT_FUNCTIONS: &[ElementFn] = &[div_element, p_element, span_element];

// #[test]
// fn test_static_list() {
//     for func in ELEMENT_FUNCTIONS {
//         func("<div>");
//     }

// }

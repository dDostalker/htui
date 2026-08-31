//! `#[derive(IntoJs)]` / `#[derive(FromJs)]` expansion.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, Lit, Meta, parse_macro_input};

use crate::compile_err;

/// `#[derive(IntoJs)]` expansion — struct → JS object.
pub(crate) fn expand_into_js(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let Some(props) = struct_field_names(&input, "IntoJs derive") else {
        return compile_err("IntoJs derive only supports named-field structs").into();
    };
    // Convert every field to a `JsValue` first (each needs `&mut __ctx`),
    // *then* build the object — `ObjectInitializer::new` borrows `__ctx`, so
    // interleaving field conversion with property insertion would double-borrow.
    let conversions: Vec<TokenStream2> = props
        .iter()
        .enumerate()
        .map(|(i, (_key, field))| {
            let v = format_ident!("__v{i}");
            quote! { let #v = self.#field.into_js(__ctx)?; }
        })
        .collect();
    let inserts: Vec<TokenStream2> = props
        .iter()
        .enumerate()
        .map(|(i, (key, _field))| {
            let v = format_ident!("__v{i}");
            quote! {
                __init.property(
                    ::htui::boa_engine::JsString::from(#key),
                    #v,
                    ::htui::boa_engine::property::Attribute::all(),
                );
            }
        })
        .collect();
    quote! {
        impl ::htui::script::IntoJs for #name {
            fn into_js(
                self,
                __ctx: &mut ::htui::boa_engine::Context,
            ) -> ::htui::boa_engine::JsResult<::htui::boa_engine::JsValue> {
                #( #conversions )*
                let mut __init =
                    ::htui::boa_engine::object::ObjectInitializer::new(__ctx);
                #( #inserts )*
                Ok(__init.build().into())
            }
        }
    }
    .into()
}

/// `#[derive(FromJs)]` expansion — JS object → struct. Field keys can be
/// renamed with `#[js_name = "..."]`, mirroring [`IntoJs`].
pub(crate) fn expand_from_js(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let Some(props) = struct_field_names(&input, "FromJs derive") else {
        return compile_err("FromJs derive only supports named-field structs").into();
    };
    let props: Vec<TokenStream2> = props
        .into_iter()
        .map(|(key, field)| {
            quote! {
                #field: {
                    let __raw =
                        __obj.get(::htui::boa_engine::JsString::from(#key), __ctx)?;
                    ::htui::script::FromJs::from_js(&__raw, __ctx)?
                }
            }
        })
        .collect();
    quote! {
        impl ::htui::script::FromJs for #name {
            fn from_js(
                __value: &::htui::boa_engine::JsValue,
                __ctx: &mut ::htui::boa_engine::Context,
            ) -> ::htui::boa_engine::JsResult<Self> {
                let __obj = __value.as_object().ok_or_else(|| {
                    ::htui::boa_engine::JsNativeError::typ()
                        .with_message("expected a JS object")
                })?;
                Ok(Self {
                    #( #props, )*
                })
            }
        }
    }
    .into()
}

/// `(js_key, field_ident)` pairs for a named-field struct.
fn struct_field_names(input: &DeriveInput, _hint: &str) -> Option<Vec<(String, Ident)>> {
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => return None,
        },
        _ => return None,
    };
    let mut out = Vec::new();
    for field in fields {
        let Some(fident) = &field.ident else { continue };
        let key = field
            .attrs
            .iter()
            .find_map(|attr| {
                if !attr.path().is_ident("js_name") {
                    return None;
                }
                match &attr.meta {
                    Meta::NameValue(nv) => match &nv.value {
                        syn::Expr::Lit(lit) => match &lit.lit {
                            Lit::Str(s) => Some(s.value()),
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                }
            })
            .unwrap_or_else(|| fident.to_string());
        out.push((key, fident.clone()));
    }
    Some(out)
}

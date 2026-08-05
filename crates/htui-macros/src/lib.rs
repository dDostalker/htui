//! Procedural macros for tauri-style host bindings.
//!
//! - `#[derive(IntoJs)]` — turn a named-field struct into a JS object
//!   (`field` → property, optionally renamed with `#[js_name = "..."]`).
//! - `#[derive(FromJs)]` — read a struct back from a JS object.
//! - `#[htui::command]` — wrap a plain Rust function (args `FromJs`,
//!   return `IntoJs`) into an htui [`HostFn`], so registering a command is
//!   just `bindings.command("name", name_command())`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, FnArg, Ident, ItemFn, Lit, Meta, Pat, ReturnType, Type,
    parse_macro_input,
};

/// `#[derive(IntoJs)]` — struct → JS object.
#[proc_macro_derive(IntoJs, attributes(js_name))]
pub fn derive_into_js(input: TokenStream) -> TokenStream {
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
                    ::htui::boa_engine::js_string!(#key),
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

/// `#[derive(FromJs)]` — JS object → struct. Field keys can be renamed with
/// `#[js_name = "..."]`, mirroring [`IntoJs`].
#[proc_macro_derive(FromJs, attributes(js_name))]
pub fn derive_from_js(input: TokenStream) -> TokenStream {
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
                        __obj.get(::htui::boa_engine::js_string!(#key), __ctx)?;
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

/// `#[htui::command]` — wrap `fn name(a: T, ...) -> R` as a `HostFn`.
#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let name = &func.sig.ident;
    let wrapper = format_ident!("{}_command", name);

    let mut arg_idents: Vec<Ident> = Vec::new();
    let mut arg_tys: Vec<Type> = Vec::new();
    for arg in &func.sig.inputs {
        match arg {
            FnArg::Typed(pat_ty) => match &*pat_ty.pat {
                Pat::Ident(pi) => {
                    arg_idents.push(pi.ident.clone());
                    arg_tys.push((*pat_ty.ty).clone());
                }
                _ => {
                    return syn::Error::new_spanned(
                        pat_ty,
                        "command args must be plain `name: Type` parameters",
                    )
                    .to_compile_error()
                    .into()
                }
            },
            FnArg::Receiver(_) => {
                return syn::Error::new_spanned(&func.sig, "command cannot be a method")
                    .to_compile_error()
                    .into()
            }
        }
    }

    let ret_ty = match &func.sig.output {
        ReturnType::Type(_, ty) => (**ty).clone(),
        ReturnType::Default => syn::parse_quote!(()),
    };
    let _ = ret_ty; // `IntoJs` on the return value handles `Result<_, E>` too.

    let arg_slots: Vec<TokenStream2> = arg_idents
        .iter()
        .enumerate()
        .map(|(i, ident)| {
            let ty = &arg_tys[i];
            quote! {
                let #ident: #ty = match ::htui::script::FromJs::from_js(
                    __call.args.get(#i).unwrap_or(&::htui::boa_engine::JsValue::undefined()),
                    __ctx,
                ) {
                    Ok(v) => v,
                    Err(e) => return Err(e),
                };
            }
        })
        .collect();

    quote! {
        #func

        /// `HostFn` wrapper for [`#name`] generated by `#[htui::command]`.
        pub fn #wrapper() -> ::htui::script::HostFn {
            Box::new(
                |__call: &mut ::htui::script::HostCall|
                    -> ::htui::boa_engine::JsResult<::htui::boa_engine::JsValue> {
                        let __ctx = &mut *__call.context;
                        #( #arg_slots )*
                        let __result = #name(#(#arg_idents),*);
                        ::htui::script::IntoJs::into_js(__result, __ctx)
                    },
            )
        }
    }
    .into()
}

/// `(js_key, field_ident)` pairs for a named-field struct.
fn struct_field_names(input: &DeriveInput, hint: &str) -> Option<Vec<(String, Ident)>> {
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
        let _ = hint;
        out.push((key, fident.clone()));
    }
    Some(out)
}

fn compile_err(msg: &str) -> TokenStream2 {
    quote! { compile_error!(#msg); }
}

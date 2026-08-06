//! Procedural macros for tauri-style host bindings.
//!
//! - `#[derive(IntoJs)]` — turn a named-field struct into a JS object
//!   (`field` → property, optionally renamed with `#[js_name = "..."]`).
//! - `#[derive(FromJs)]` — read a struct back from a JS object.
//! - `#[htui::command]` — the only way to expose a Rust function to JS:
//!   - a plain function (args `FromJs`, return `IntoJs`) is wrapped into a
//!     [`HostFn`] (`name_command()`), or
//!   - a zero-argument function returning a [`HostFn`] is registered as-is,
//!     letting the returned closure capture arbitrary Rust state.
//!   Accepts an optional `name = "..."` to control the JS-side name used by
//!   `#[htui::commands]`.
//! - `#[htui::commands]` — collect every `#[htui::command]` function in a
//!   module and generate one `register(&mut EventBindings)` entry point (plus
//!   an optional auto-refresh), removing the per-command `bindings.command(...)`
//!   boilerplate.

use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Fields, FnArg, Ident, Item, ItemFn, ItemMod,
    Lit, LitInt, LitStr, Meta, Pat, ReturnType, Type, parse_macro_input, parse_quote,
    parse::{Parse, ParseStream, Parser},
    punctuated::Punctuated,
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

/// `#[htui::command]` — the single way to expose a Rust function to JS.
///
/// A plain function (args `FromJs`, return `IntoJs`) is wrapped into a
/// `HostFn`; a zero-argument function returning a `HostFn` is the stateful
/// variant — the returned closure (which may capture state) becomes the host
/// function directly. Accepts an optional `name = "..."` attribute that
/// `#[htui::commands]` uses as the JS registration name.
#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    if let Err(e) = command_attr(attr) {
        return e.to_compile_error().into();
    }
    let func = parse_macro_input!(item as ItemFn);
    let name = &func.sig.ident;
    let wrapper = format_ident!("{}_command", name);

    // A function returning `HostFn` is a stateful command: the closure it
    // returns (capturing whatever state it needs) is the host function.
    if returns_host_fn(&func.sig.output) {
        if !func.sig.inputs.is_empty() {
            return syn::Error::new_spanned(
                &func.sig,
                "a `#[htui::command]` returning `HostFn` must take no arguments",
            )
            .to_compile_error()
            .into();
        }
        return quote! {
            #func

            /// `HostFn` wrapper for [`#name`] generated by `#[htui::command]`.
            pub fn #wrapper() -> ::htui::script::HostFn {
                #name()
            }
        }
        .into();
    }

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

/// Whether the function returns a `HostFn` (matched on the last path segment,
/// so `HostFn`, `htui::script::HostFn` and `::htui::script::HostFn` all work).
fn returns_host_fn(output: &ReturnType) -> bool {
    match output {
        ReturnType::Type(_, ty) => match &**ty {
            Type::Path(tp) => tp
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "HostFn"),
            _ => false,
        },
        ReturnType::Default => false,
    }
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

/// Validate the optional `name = "…"` attribute on `#[htui::command]`.
///
/// The value is read by [`commands`] when it registers the command; a
/// bare `#[htui::command]` (no attribute) is also valid.
fn command_attr(attr: TokenStream) -> syn::Result<()> {
    let metas = Punctuated::<Meta, syn::token::Comma>::parse_terminated.parse2(attr.into())?;
    for meta in metas {
        match meta {
            Meta::NameValue(nv) if nv.path.is_ident("name") => match &nv.value {
                Expr::Lit(ExprLit { lit: Lit::Str(_), .. }) => {}
                _ => {
                    return Err(syn::Error::new_spanned(
                        nv,
                        "`name` must be a string literal, e.g. #[htui::command(name = \"jsName\")]",
                    ));
                }
            },
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "unsupported `#[htui::command]` attribute; expected `name = \"...\"`",
                ));
            }
        }
    }
    Ok(())
}

/// `#[htui::commands]` — collect every `#[htui::command]` in a module and
/// generate a single `register(&mut EventBindings)` entry point.
///
/// Applied to a module, the macro scans its items for `#[htui::command]`
/// functions and registers each under its function name (or the name given by
/// `#[htui::command(name = "...")]`), whether the command is a plain function
/// or a stateful one returning a [`HostFn`] — the registration is uniform.
///
/// The attribute also accepts `auto = "action"` plus `every_ms = N` to
/// configure the periodic auto-refresh:
///
/// ```ignore
/// #[htui::commands(auto = "refresh", every_ms = 200)]
/// mod bindings {
///     #[htui::command]
///     fn bar(ratio: f32, width: usize) -> String { /* … */ }
///
///     // Stateful: the returned closure captures what it needs.
///     #[htui::command]
///     fn sample() -> HostFn { /* … */ }
/// }
///
/// let mut bindings = EventBindings::default();
/// bindings::register(&mut bindings);
/// ```
#[proc_macro_attribute]
pub fn commands(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as CommandsAttr);
    let mut module = parse_macro_input!(item as ItemMod);

    let registrations = match command_registrations(&module) {
        Ok(r) => r,
        Err(e) => return e.to_compile_error().into(),
    };

    let auto_setup = match (&attr.auto, attr.every_ms) {
        (Some(action), Some(every_ms)) => quote! {
            bindings.auto = Some(::htui::events::AutoRefresh {
                every: ::std::time::Duration::from_millis(#every_ms),
                action: #action.to_string(),
            });
        },
        _ => quote! {},
    };

    let Some((_, items)) = module.content.as_mut() else {
        return syn::Error::new_spanned(
            &module,
            "`#[htui::commands]` requires a module with an inline body",
        )
        .to_compile_error()
        .into();
    };
    items.push(parse_quote! {
        /// Register every `#[htui::command]` in this module (plus the optional
        /// auto-refresh) into `bindings`.
        pub fn register(bindings: &mut ::htui::events::EventBindings) {
            #( #registrations )*
            #auto_setup
        }
    });
    quote! { #module }.into()
}

/// Attribute arguments for `#[htui::commands]` (`auto = "…"` + `every_ms = N`).
struct CommandsAttr {
    auto: Option<String>,
    every_ms: Option<u64>,
}

impl Parse for CommandsAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut auto = None;
        let mut every_ms = None;
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<syn::token::Eq>()?;
            match key.to_string().as_str() {
                "auto" => {
                    let lit: LitStr = input.parse()?;
                    auto = Some(lit.value());
                }
                "every_ms" => {
                    let lit: LitInt = input.parse()?;
                    every_ms = Some(lit.base10_parse()?);
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        key,
                        format!(
                            "unknown `#[htui::commands]` argument `{other}`; \
                             expected `auto = \"...\"` and/or `every_ms = N`"
                        ),
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<syn::token::Comma>()?;
            }
        }
        if auto.is_some() != every_ms.is_some() {
            return Err(syn::Error::new(
                input.span(),
                "`#[htui::commands]`: `auto` and `every_ms` must be used together",
            ));
        }
        Ok(Self { auto, every_ms })
    }
}

/// Scan a `#[htui::commands]` module for `#[htui::command]` functions and
/// build their registration statements (uniform: every command registers
/// through its generated `<fn>_command()` wrapper).
fn command_registrations(module: &ItemMod) -> syn::Result<Vec<TokenStream2>> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let Some((_, items)) = &module.content else {
        return Err(syn::Error::new_spanned(
            module,
            "`#[htui::commands]` requires a module with an inline body",
        ));
    };
    for item in items {
        let Item::Fn(func) = item else { continue };
        let fn_name = &func.sig.ident;

        // The attribute is written `#[htui::command]` (or a bare `#[command]`
        // after `use htui::command;`), so match on the last path segment.
        let is_command = |a: &&Attribute| {
            a.path().segments.last().is_some_and(|s| s.ident == "command")
        };
        let Some(command_attr) = func.attrs.iter().find(|a| is_command(a)) else {
            continue;
        };

        let js_name = command_js_name(command_attr, fn_name)?;
        let wrapper = format_ident!("{}_command", fn_name);
        if !seen.insert(js_name.clone()) {
            return Err(syn::Error::new_spanned(
                func,
                format!("duplicate command registration name `{js_name}`"),
            ));
        }
        out.push(quote! {
            bindings.command(#js_name, #wrapper());
        });
    }
    Ok(out)
}

/// JS registration name for a `#[htui::command]` function: the `name = "…"`
/// attribute value, or the function name when no override is given.
fn command_js_name(attr: &Attribute, fallback: &Ident) -> syn::Result<String> {
    // A bare `#[htui::command]` carries no arguments → use the fn name.
    if matches!(&attr.meta, Meta::Path(_)) {
        return Ok(fallback.to_string());
    }
    let metas = attr.parse_args_with(Punctuated::<Meta, syn::token::Comma>::parse_terminated)?;
    for meta in metas {
        match meta {
            Meta::NameValue(nv) if nv.path.is_ident("name") => match &nv.value {
                Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => return Ok(s.value()),
                _ => {
                    return Err(syn::Error::new_spanned(
                        nv,
                        "`name` must be a string literal, e.g. #[htui::command(name = \"jsName\")]",
                    ));
                }
            },
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "unsupported `#[htui::command]` attribute; expected `name = \"...\"`",
                ));
            }
        }
    }
    Ok(fallback.to_string())
}

fn compile_err(msg: &str) -> TokenStream2 {
    quote! { compile_error!(#msg); }
}

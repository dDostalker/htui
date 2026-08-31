//! `#[htui::commands]` attribute macro expansion: module-wide registration
//! plus the auto-generated sample JS interface.

use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro2::{Literal, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ExprLit, FnArg, GenericArgument, Ident, Item, ItemMod, Lit, Meta, Pat,
    PathArguments, ReturnType, Type, parse_macro_input, parse_quote, punctuated::Punctuated,
};

use crate::command::returns_host_fn;

/// `#[htui::commands]` expansion — collect every `#[htui::command]` in a
/// module and generate a single `register(&mut EventBindings)` entry point.
///
/// Applied to a module, the macro scans its items for `#[htui::command]`
/// functions and registers each under its function name (or the name given by
/// `#[htui::command(name = "...")]`), whether the command is a plain function
/// or a stateful one returning a [`HostFn`] — the registration is uniform.
///
/// In addition to `register`, the expansion generates `js_bindings()` — an
/// auto-generated sample JS interface for the module's plain commands
/// (JSDoc-annotated function declarations that code analyzers can read) — and
/// `write_js_bindings(path)` to write it to a file. Stateful commands
/// (`fn() -> HostFn`) are excluded (no static signature).
pub(crate) fn expand_commands(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "`#[htui::commands]` takes no arguments: the former `auto = \"...\"` / \
             `every_ms = N` refresh was removed — schedule it from the page with \
             `setInterval(fn, ms)` instead",
        )
        .to_compile_error()
        .into();
    }
    let mut module = parse_macro_input!(item as ItemMod);

    let registrations = match command_registrations(&module) {
        Ok(r) => r,
        Err(e) => return e.to_compile_error().into(),
    };
    let js_bindings = match command_js_bindings(&module) {
        Ok(b) => b,
        Err(e) => return e.to_compile_error().into(),
    };
    let bindings_lit = Literal::string(&js_bindings);

    let Some((_, items)) = module.content.as_mut() else {
        return syn::Error::new_spanned(
            &module,
            "`#[htui::commands]` requires a module with an inline body",
        )
        .to_compile_error()
        .into();
    };
    items.push(parse_quote! {
        /// Register every `#[htui::command]` in this module into `bindings`.
        pub fn register(bindings: &mut ::htui::events::EventBindings) {
            #( #registrations )*
        }
    });
    items.push(parse_quote! {
        /// Auto-generated sample JS interface for this module's plain
        /// commands.
        ///
        /// Returns the full source as a `&'static str`: one JSDoc-annotated
        /// `function` declaration per command, so code analyzers (VS Code,
        /// tsserver, AI tooling) can read the JS-side surface without running
        /// anything. Stateful commands (`fn() -> HostFn`) have no static
        /// signature and are skipped.
        pub fn js_bindings() -> &'static str {
            #bindings_lit
        }
    });
    items.push(parse_quote! {
        /// Write the auto-generated sample JS interface (see [`js_bindings`])
        /// to `path`.
        pub fn write_js_bindings(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
            std::fs::write(path, #bindings_lit)
        }
    });
    quote! { #module }.into()
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

/// JavaScript type for a `#[htui::command]` signature type.
///
/// Known Rust ↔ JS mappings (mirroring `IntoJs`/`FromJs`); unknown types fall
/// back to `any`.
fn js_type(ty: &Type) -> String {
    let Type::Path(tp) = ty else {
        if let Type::Tuple(t) = ty
            && t.elems.is_empty()
        {
            return "void".to_string();
        }
        return "any".to_string();
    };
    let Some(seg) = tp.path.segments.last() else {
        return "any".to_string();
    };
    let inner = |index: usize| -> String {
        match &seg.arguments {
            PathArguments::AngleBracketed(args) => args
                .args
                .iter()
                .filter_map(|a| match a {
                    GenericArgument::Type(t) => Some(js_type(t)),
                    _ => None,
                })
                .nth(index)
                .unwrap_or_else(|| "any".to_string()),
            _ => "any".to_string(),
        }
    };
    match seg.ident.to_string().as_str() {
        "String" | "str" => "string".to_string(),
        "f64" | "f32" | "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64"
        | "isize" | "usize" => "number".to_string(),
        "bool" => "boolean".to_string(),
        "Option" => format!("{} | undefined", inner(0)),
        "Vec" => format!("{}[]", inner(0)),
        // `Result<T, E>` returns `T`; the error is thrown as a JS exception.
        "Result" => inner(0),
        _ => "any".to_string(),
    }
}

/// Whether the return type is a `Result` — the JS side sees the `Ok` value or
/// a thrown exception.
fn is_result_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Path(tp) if tp.path.segments.last().is_some_and(|s| s.ident == "Result")
    )
}

/// Build the auto-generated sample JS interface for every *plain* command in a
/// `#[htui::commands]` module. Stateful commands (`fn() -> HostFn`) carry no
/// static signature and are skipped.
///
/// The output is one JSDoc-annotated `function` declaration per command — a
/// readable contract that code analyzers (VS Code, tsserver, AI tooling) can
/// understand without running anything.
fn command_js_bindings(module: &ItemMod) -> syn::Result<String> {
    let mut decls = String::new();

    let Some((_, items)) = &module.content else {
        return Err(syn::Error::new_spanned(
            module,
            "`#[htui::commands]` requires a module with an inline body",
        ));
    };
    for item in items {
        let Item::Fn(func) = item else { continue };
        let fn_name = &func.sig.ident;

        let is_command = |a: &&Attribute| {
            a.path().segments.last().is_some_and(|s| s.ident == "command")
        };
        let Some(command_attr) = func.attrs.iter().find(|a| is_command(a)) else {
            continue;
        };
        if returns_host_fn(&func.sig.output) {
            continue;
        }
        let js_name = command_js_name(command_attr, fn_name)?;

        let mut args = Vec::new();
        for (i, input) in func.sig.inputs.iter().enumerate() {
            let FnArg::Typed(pat_ty) = input else { continue };
            let name = match &*pat_ty.pat {
                Pat::Ident(pi) => pi.ident.to_string(),
                _ => format!("arg{i}"),
            };
            args.push((name, js_type(&pat_ty.ty)));
        }

        let ret = match &func.sig.output {
            ReturnType::Type(_, ty) => js_type(ty),
            ReturnType::Default => "void".to_string(),
        };
        let throws = matches!(&func.sig.output, ReturnType::Type(_, ty) if is_result_type(ty));

        decls.push_str("/**\n");
        for (name, ty) in &args {
            decls.push_str(&format!(" * @param {{{ty}}} {name}\n"));
        }
        decls.push_str(&format!(" * @returns {{{ret}}}\n"));
        if throws {
            decls.push_str(" * Throws on error.\n");
        }
        decls.push_str(" */\n");
        let arg_names = args
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        decls.push_str(&format!("function {js_name}({arg_names}) {{}}\n\n"));
    }

    let mut out = String::new();
    out.push_str(
        "// htui sample JS interface — auto-generated by `#[htui::commands]`. Do not edit.\n",
    );
    out.push_str("// Code analyzers (VS Code, tsserver, AI tooling) read these declarations to\n");
    out.push_str("// understand the JS globals the Rust host registers. The bodies are empty on\n");
    out.push_str("// purpose: the real implementations live in the Rust host. Stateful commands\n");
    out.push_str("// (`fn() -> HostFn`) have no static signature and are skipped.\n\n");
    out.push_str(&decls);
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

        if let Some(meta) = metas.into_iter().next() {
                match meta{
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

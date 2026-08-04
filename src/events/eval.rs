//! Minimal JavaScript-subset evaluator for `on*` event handler bodies.
//!
//! Browsers compile `onclick="…"` attribute values into an event handler
//! function and execute them as JavaScript. This module implements a small,
//! safe subset of that behaviour so the same attribute semantics apply here:
//! the attribute value is *code*, not a label.
//!
//! Supported syntax (v1):
//! - bare identifiers (`onclick="buy"`) — resolved but never invoked, exactly
//!   like the browser's `function (event) { buy }`;
//! - call expressions (`onclick="buy()"`, `onclick="console.log('x')"`);
//! - string / number literals as arguments;
//! - dotted member access for the `console.*` host object;
//! - multiple statements separated by `;`.
//!
//! Calls resolve against the [`EventBindings::actions`] registry first — the
//! application's global scope — then fall back to the built-ins (`alert`,
//! `quit`). `console.*` is reserved. Unparseable bodies and unknown functions
//! log a warning and are skipped, mirroring the browser's silent
//! `ReferenceError` failure mode without crashing the renderer.

use crate::tui::graph_api::RenderCmd;

use super::{ElementRef, EventBindings, EventType, HtmlEvent};

/// Everything dispatch knows about an event before the handler body runs.
///
/// The evaluator turns this context into an [`HtmlEvent`] for every function
/// invocation the body makes.
pub struct EventContext<'a> {
    /// The event being dispatched.
    pub event_type: EventType,
    /// `id` of the deepest element under the pointer (the original target).
    pub target: Option<&'a str>,
    /// `id` of the element whose `on*` body is being evaluated.
    pub current_target: Option<&'a str>,
    /// Full bubbling chain: target first, then ancestors up to the root.
    pub path: &'a [ElementRef],
    /// Canvas coordinates (already offset by `scroll_y`).
    pub x: u32,
    pub y: u32,
}

/// Evaluate an `on*` handler body, invoking the functions it calls.
///
/// Returns the first [`RenderCmd`] produced by an invoked handler or built-in,
/// or `None` when the body makes no call, calls nothing registered, or fails
/// to parse.
pub fn eval_handler(
    body: &str,
    ctx: &EventContext<'_>,
    bindings: &mut EventBindings,
) -> Option<RenderCmd> {
    let program = match parse(body) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Failed to parse event handler `{body}`: {e}");
            return None;
        }
    };
    for stmt in &program {
        if let Some(cmd) = eval_stmt(stmt, ctx, bindings) {
            return Some(cmd);
        }
    }
    None
}

// ============================================================
// AST + parser
// ============================================================

#[derive(Debug)]
enum Stmt {
    /// A bare reference that resolves without invoking (`onclick="buy"`).
    Bare(Path),
    /// A call expression such as `buy()` or `console.log('x')`.
    Call(Path, Vec<Arg>),
}

/// Dotted function reference, e.g. `["console", "log"]` or `["buy"]`.
#[derive(Debug)]
struct Path(Vec<String>);

#[derive(Debug)]
enum Arg {
    Str(String),
    Num(f64),
    Ident(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Num(f64),
    Dot,
    LParen,
    RParen,
    Comma,
    Semi,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn is_ident_continue(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }
            ';' => {
                chars.next();
                tokens.push(Token::Semi);
            }
            '(' => {
                chars.next();
                tokens.push(Token::LParen);
            }
            ')' => {
                chars.next();
                tokens.push(Token::RParen);
            }
            ',' => {
                chars.next();
                tokens.push(Token::Comma);
            }
            '.' => {
                chars.next();
                tokens.push(Token::Dot);
            }
            '"' | '\'' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some(ch) if ch == c => break,
                        Some('\\') => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('\\') => s.push('\\'),
                            Some('\'') => s.push('\''),
                            Some('"') => s.push('"'),
                            Some(other) => s.push(other),
                            None => return Err("unterminated string literal".to_string()),
                        },
                        Some(ch) => s.push(ch),
                        None => return Err("unterminated string literal".to_string()),
                    }
                }
                tokens.push(Token::Str(s));
            }
            _ if c.is_ascii_digit() => {
                let mut num = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        num.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let value: f64 = num
                    .parse()
                    .map_err(|_| format!("invalid number `{num}`"))?;
                tokens.push(Token::Num(value));
            }
            _ if is_ident_start(c) => {
                let mut name = String::new();
                while let Some(&ch) = chars.peek() {
                    if is_ident_continue(ch) {
                        name.push(ch);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Ident(name));
            }
            other => return Err(format!("unexpected character `{other}`")),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn expect(&mut self, token: &Token) -> Result<(), String> {
        match self.next() {
            Some(t) if &t == token => Ok(()),
            other => Err(format!("expected `{token:?}`, found `{other:?}`")),
        }
    }

    fn program(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        loop {
            while matches!(self.peek(), Some(Token::Semi)) {
                self.next();
            }
            if self.peek().is_none() {
                break;
            }
            stmts.push(self.statement()?);
        }
        Ok(stmts)
    }

    fn statement(&mut self) -> Result<Stmt, String> {
        let path = self.path()?;
        if matches!(self.peek(), Some(Token::LParen)) {
            self.next();
            let mut args = Vec::new();
            if !matches!(self.peek(), Some(Token::RParen)) {
                loop {
                    args.push(self.arg()?);
                    if matches!(self.peek(), Some(Token::Comma)) {
                        self.next();
                    } else {
                        break;
                    }
                }
            }
            self.expect(&Token::RParen)?;
            Ok(Stmt::Call(path, args))
        } else {
            Ok(Stmt::Bare(path))
        }
    }

    fn path(&mut self) -> Result<Path, String> {
        let first = match self.next() {
            Some(Token::Ident(name)) => name,
            other => return Err(format!("expected identifier, found `{other:?}`")),
        };
        let mut parts = vec![first];
        while matches!(self.peek(), Some(Token::Dot)) {
            self.next();
            match self.next() {
                Some(Token::Ident(name)) => parts.push(name),
                other => return Err(format!("expected identifier after `.`, found `{other:?}`")),
            }
        }
        Ok(Path(parts))
    }

    fn arg(&mut self) -> Result<Arg, String> {
        match self.next() {
            Some(Token::Str(s)) => Ok(Arg::Str(s)),
            Some(Token::Num(n)) => Ok(Arg::Num(n)),
            Some(Token::Ident(name)) => Ok(Arg::Ident(name)),
            other => Err(format!("expected argument, found `{other:?}`")),
        }
    }
}

fn parse(src: &str) -> Result<Vec<Stmt>, String> {
    let tokens = tokenize(src)?;
    let mut parser = Parser { tokens, pos: 0 };
    parser.program()
}

// ============================================================
// Evaluator
// ============================================================

fn eval_stmt(
    stmt: &Stmt,
    ctx: &EventContext<'_>,
    bindings: &mut EventBindings,
) -> Option<RenderCmd> {
    match stmt {
        // A bare reference resolves but never invokes — matching the
        // browser's `function (event) { buy }` no-op. Like the browser's
        // `ReferenceError` for undeclared globals, an unknown identifier is
        // reported as a warning and then skipped.
        Stmt::Bare(path) => {
            if let Some(head) = path.0.first()
                && !bindings.actions.contains_key(head)
                && !matches!(head.as_str(), "console" | "alert" | "quit")
            {
                log::warn!("Unknown identifier `{head}` in event handler");
            }
            None
        }
        Stmt::Call(path, args) => match path.0.as_slice() {
            [base, member] if base == "console" => {
                log_console(member, args);
                None
            }
            [name] => {
                // The application registry (global scope) shadows the
                // built-ins, mirroring user globals in the browser.
                if let Some(handler) = bindings.actions.get_mut(name) {
                    let event = HtmlEvent {
                        event_type: ctx.event_type,
                        target: ctx.target,
                        current_target: ctx.current_target,
                        path: ctx.path,
                        action: Some(name.as_str()),
                        x: ctx.x,
                        y: ctx.y,
                        bubbles: true,
                    };
                    return handler(&event);
                }
                match name.as_str() {
                    "alert" => {
                        log::warn!("[alert] {}", stringify_args(args));
                        None
                    }
                    "quit" => Some(RenderCmd::Quit),
                    _ => {
                        log::warn!("Unknown function `{name}` in event handler");
                        None
                    }
                }
            }
            other => {
                log::warn!(
                    "Unsupported function reference `{}` in event handler",
                    other.join(".")
                );
                None
            }
        },
    }
}

/// `console.*` host methods — logged to the htui logger instead of a terminal
/// console.
fn log_console(method: &str, args: &[Arg]) {
    let msg = stringify_args(args);
    match method {
        "log" => log::info!("[console.log] {msg}"),
        "warn" => log::warn!("[console.warn] {msg}"),
        "error" => log::error!("[console.error] {msg}"),
        _ => log::info!("[console.{method}] {msg}"),
    }
}

/// Render the argument list as a human-readable string.
fn stringify_args(args: &[Arg]) -> String {
    args.iter()
        .map(|a| match a {
            Arg::Str(s) => s.clone(),
            Arg::Num(n) => n.to_string(),
            Arg::Ident(name) => name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

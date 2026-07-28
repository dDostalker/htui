use cssparser::{AtRuleParser, BasicParseErrorKind, CowRcStr, Delimiter, ParseError, Parser as CssParser, ParserInput, ParserState, QualifiedRuleParser, StyleSheetParser, parse_important};
use indexmap::IndexMap;
use log::{debug, warn};

use super::{properties::CssAttr, selector::CssSelector};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct CssRule(pub IndexMap<CssSelector, CssAttr>);

impl CssRule {
    pub fn new() -> Self {
        CssRule(IndexMap::new())
    }
    pub fn add(&mut self, sel: CssSelector, attr: IndexMap<String, String>) {
        self.0.insert(sel, CssAttr(attr));
    }
}

// Parser

#[derive(Debug, Clone)]
pub struct Parser {
    context: String,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        Parser {
            context: input.to_string(),
        }
    }

    pub fn parse(&mut self) -> CssRule {
        let mut parser_input = ParserInput::new(&self.context);
        let mut parser = CssParser::new(&mut parser_input);
        let mut state = CssParseState::new();
        {
            let rule_list = StyleSheetParser::new(&mut parser, &mut state);
            for rule_result in rule_list {
                if let std::result::Result::Err(e) = rule_result {
                    warn!("CSS error: {:?}", e);
                }
            }
        }
        let mut css = CssRule::new();
        for (sel_text, declarations) in state.rules.drain(..) {
            match CssSelector::from_str(&sel_text) {
                std::result::Result::Ok(sel) => css.add(sel, declarations),
                std::result::Result::Err(e) => warn!("Selector parse fail: {}: {}", sel_text, e),
            }
        }
        debug!("CSS parsed: {} rules", css.0.len());
        css
    }
}

struct CssParseState {
    rules: Vec<(String, IndexMap<String, String>)>,
}
impl CssParseState {
    fn new() -> Self {
        Self { rules: vec![] }
    }
}

impl<'i> QualifiedRuleParser<'i> for CssParseState {
    type Prelude = String;
    type QualifiedRule = ();
    type Error = cssparser::BasicParseError<'i>;
    fn parse_prelude<'t>(
        &mut self,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<String, ParseError<'i, Self::Error>> {
        let start = input.position();
        while !input.is_exhausted() {
            if input.next_including_whitespace_and_comments().is_err() {
                break;
            }
        }
        let text = input.slice_from(start).trim().to_string();
        if text.is_empty() {
            return Err(input.new_error(BasicParseErrorKind::QualifiedRuleInvalid));
        }
        Ok(text)
    }
    fn parse_block<'t>(
        &mut self,
        prelude: String,
        _start: &ParserState,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<(), ParseError<'i, Self::Error>> {
        let mut declarations = IndexMap::new();
        parse_declarations_into(input, &mut declarations);
        self.rules.push((prelude, declarations));
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for CssParseState {
    type Prelude = ();
    type AtRule = ();
    type Error = cssparser::BasicParseError<'i>;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<(), ParseError<'i, Self::Error>> {
        debug!("skip @{}", name);
        while !input.is_exhausted() {
            input.next().ok();
        }
        Ok(())
    }
    fn parse_block<'t>(
        &mut self,
        _: (),
        _: &ParserState,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<(), ParseError<'i, Self::Error>> {
        while !input.is_exhausted() {
            input.next().ok();
        }
        Ok(())
    }
    fn rule_without_block(&mut self, _: (), _: &ParserState) -> std::result::Result<(), ()> {
        Ok(())
    }
}

/// Trim a declaration value and strip any `!important` token that survived
/// token-level parsing (defensive — the parser usually consumes it).
fn clean_value_text(raw: &str) -> String {
    let trimmed = raw.trim();
    match trimmed.find("!important") {
        Some(idx) => trimmed[..idx].trim_end().to_string(),
        None => trimmed.to_string(),
    }
}

/// Parse a CSS declaration block text into key-value pairs.
///
/// Built on top of [`cssparser`]'s tokenizer via [`parse_declarations_into`]
/// so it correctly handles:
/// - `/* ... */` block comments (including those inside string literals)
/// - the `!important` marker
/// - missing trailing `;`
/// - escaped identifiers and unicode in property names
/// - quoted string values with semicolons / colons inside
#[doc(hidden)]
pub fn parse_declaration_text(block: &str) -> IndexMap<String, String> {
    let mut input_buf = ParserInput::new(block);
    let mut input = CssParser::new(&mut input_buf);
    let mut map = IndexMap::new();
    parse_declarations_into(&mut input, &mut map);
    map
}

/// Walk every `name: value [!important]` declaration in `input` and store
/// them in `out`. Tolerates malformed input by skipping to the next `;`
/// rather than aborting the whole block.
fn parse_declarations_into<'i, 't>(
    input: &mut CssParser<'i, 't>,
    out: &mut IndexMap<String, String>,
) {
    loop {
        input.skip_whitespace();
        if input.is_exhausted() {
            return;
        }
        let name = match input.expect_ident() {
            Ok(s) => s.clone(),
            Err(_) => {
                // Malformed declaration (e.g. starts with `;` or unknown
                // token). Skip ahead to the next semicolon to avoid an
                // infinite loop on the same junk token.
                let _: Result<(), ParseError<()>> = input.parse_until_after(
                    Delimiter::Semicolon,
                    |_| Ok(()),
                );
                continue;
            }
        };
        if input.expect_colon().is_err() {
            // `name` without `:` — advance past the bogus declaration.
            let _: Result<(), ParseError<()>> = input.parse_until_after(
                Delimiter::Semicolon,
                |_| Ok(()),
            );
            continue;
        }
        let value_start = input.position();
        // Consume value tokens until `;` or `!important` is reached.
        let _: Result<(), ParseError<()>> = input.parse_until_before(
            Delimiter::Semicolon | Delimiter::Bang,
            |_| Ok(()),
        );
        let value_text = input.slice_from(value_start);
        let _important = input.try_parse(parse_important).is_ok();
        let cleaned = clean_value_text(value_text);
        if !name.is_empty() && !cleaned.is_empty() {
            out.insert(name.to_string(), cleaned);
        }
        // Consume the trailing `;` if present.
        let _ = input.try_parse(|i| i.expect_semicolon().map(|_| ()));
    }
}

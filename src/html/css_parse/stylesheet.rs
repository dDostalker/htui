use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, Delimiter, ParseError, Parser as CssParser,
    ParserInput, ParserState, QualifiedRuleParser, StyleSheetParser, parse_important,
};
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

    /// Insert a `(selector, declarations)` pair.
    ///
    /// If `sel` is already present, the new declarations are merged into the
    /// existing rule instead of overwriting it — later rules with the same
    /// selector win for any single property (matching the CSS cascade), but
    /// properties only present in the earlier rule are preserved. Without
    /// this merge, two rules like `.a, .grow { background: red }` and
    /// `.grow { flex-grow: 1 }` would leave `.grow` with only `flex-grow`,
    /// silently dropping `background`.
    pub fn add(&mut self, sel: CssSelector, attr: IndexMap<String, String>) {
        if let Some(existing) = self.0.get_mut(&sel) {
            for (k, v) in attr {
                existing.0.insert(k, v);
            }
        } else {
            self.0.insert(sel, CssAttr(attr));
        }
    }
}

impl Default for CssRule {
    fn default() -> Self {
        Self::new()
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
            // Split comma-separated selectors: `table, td { ... }` is two
            // independent rules targeting distinct node types. Each
            // sub-selector is parsed in isolation so it can stand on its
            // own when applied via the cascade.
            for sub_sel in split_top_level_commas(&sel_text) {
                match CssSelector::from_str(sub_sel) {
                    std::result::Result::Ok(sel) => css.add(sel, declarations.clone()),
                    std::result::Result::Err(e) => {
                        warn!("Selector parse fail: {}: {}", sub_sel, e);
                    }
                }
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

/// Split a comma-separated selector list at top-level commas only.
///
/// Commas inside `()` (e.g. `:not(a, b)`) are preserved so pseudo-class
/// argument lists stay intact. The returned slices are trimmed and any
/// empty entries (caused by stray commas) are dropped.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut last = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = s[last..i].trim();
                if !part.is_empty() {
                    out.push(part);
                }
                last = i + 1;
            }
            _ => {}
        }
    }
    let tail = s[last..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
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
                let _: Result<(), ParseError<()>> =
                    input.parse_until_after(Delimiter::Semicolon, |_| Ok(()));
                continue;
            }
        };
        if input.expect_colon().is_err() {
            // `name` without `:` — advance past the bogus declaration.
            let _: Result<(), ParseError<()>> =
                input.parse_until_after(Delimiter::Semicolon, |_| Ok(()));
            continue;
        }
        let value_start = input.position();
        // Consume value tokens until `;` or `!important` is reached.
        let _: Result<(), ParseError<()>> =
            input.parse_until_before(Delimiter::Semicolon | Delimiter::Bang, |_| Ok(()));
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

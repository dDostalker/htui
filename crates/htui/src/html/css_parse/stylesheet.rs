use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, Delimiter, ParseError, Parser as CssParser,
    ParserInput, ParserState, QualifiedRuleParser, StyleSheetParser, parse_important,
};
use indexmap::IndexMap;
use log::{debug, warn};

use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use super::cascade::CascadeIndex;
use super::{
    properties::CssAttr, selector::CssSelector,
    values::{
        alignment_longhand_valid, expand_place_shorthand, is_alignment_longhand,
        is_place_shorthand_name,
    },
};

/// One qualified rule, with declarations already split by the `!important`
/// marker at parse time. Rules are kept separate (no same-selector merging) so
/// the cascade can order every rule by specificity and source position exactly
/// as the stylesheet wrote it — merging identical selectors would collapse the
/// source order of interleaved rules.
#[derive(Debug, Clone)]
pub struct CssRuleEntry {
    pub selector: CssSelector,
    /// Non-`!important` declarations.
    pub normal: CssAttr,
    /// `!important` declarations.
    pub important: CssAttr,
}

/// The parsed author stylesheet: every qualified rule, with the `!important`
/// split already applied.
///
/// The second field is the lazily-built [`CascadeIndex`] — computed on the
/// first cascade pass and dropped by [`push`](Self::push) /
/// [`add`](Self::add) so the next pass rebuilds it against the extended rule
/// set. Rules mutated by any other route leave the index stale; extend the
/// rule set only through those two methods.
#[derive(Debug, Clone)]
pub struct CssRule(
    pub Vec<CssRuleEntry>,
    /// Lazily-built cascade index; see [`CssRule::cascade_index`].
    pub(crate) OnceLock<CascadeIndex>,
);

impl CssRule {
    pub fn new() -> Self {
        CssRule(Vec::new(), OnceLock::new())
    }

    /// The cascade index for this rule set, built on first use.
    pub(crate) fn cascade_index(&self) -> &CascadeIndex {
        self.1.get_or_init(|| CascadeIndex::build(self))
    }

    /// Append a rule with explicit normal / important declaration splits.
    pub fn push(&mut self, selector: CssSelector, normal: CssAttr, important: CssAttr) {
        self.1.take();
        self.0.push(CssRuleEntry {
            selector,
            normal,
            important,
        });
    }

    /// Append a rule whose declarations all have normal priority.
    pub fn add(&mut self, selector: CssSelector, attr: CssAttr) {
        self.1.take();
        self.0.push(CssRuleEntry {
            selector,
            normal: attr,
            important: CssAttr::new(),
        });
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
        for (sel_text, normal, important) in state.rules.drain(..) {
            // Split comma-separated selectors: `table, td { ... }` is two
            // independent rules targeting distinct node types. Each
            // sub-selector is parsed in isolation so it can stand on its
            // own when applied via the cascade.
            for sub_sel in split_top_level_commas(&sel_text) {
                match CssSelector::from_str(sub_sel) {
                    std::result::Result::Ok(sel) => {
                        css.push(sel, normal.clone(), important.clone())
                    }
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
    rules: Vec<(String, CssAttr, CssAttr)>,
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
        let mut normal = CssAttr::new();
        let mut important = CssAttr::new();
        parse_declarations_into(input, &mut normal, &mut important, true);
        self.rules.push((prelude, normal, important));
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
/// token-level parsing (defensive — the parser usually consumes it). Returns a
/// shared `Arc<str>` so the value is stored without a second owned copy.
fn clean_value_text(raw: &str) -> Arc<str> {
    let trimmed = raw.trim();
    if let Some(idx) = trimmed.find("!important") {
        Arc::from(trimmed[..idx].trim_end())
    } else {
        Arc::from(trimmed)
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
    if let Some(tail) = s.get(last..) {
        let tail = tail.trim();
        if !tail.is_empty() {
            out.push(tail);
        }
    }
    out
}

/// Parse a CSS declaration block text into key-value pairs (the `!important`
/// marker is stripped from the values).
///
/// Built on top of [`cssparser`]'s tokenizer via [`parse_declaration_splits`]
/// so it correctly handles:
/// - `/* ... */` block comments (including those inside string literals)
/// - the `!important` marker
/// - missing trailing `;`
/// - escaped identifiers and unicode in property names
/// - quoted string values with semicolons / colons inside
#[doc(hidden)]
pub fn parse_declaration_text(block: &str) -> IndexMap<Arc<str>, Arc<str>> {
    // No `place-*` expansion: this map backs the JS `el.style` object, whose
    // shorthand properties read back the author-specified text (the cascade
    // expands separately via [`parse_declaration_splits`]).
    let mut input_buf = ParserInput::new(block);
    let mut input = CssParser::new(&mut input_buf);
    let mut normal = CssAttr::new();
    let mut important = CssAttr::new();
    parse_declarations_into(&mut input, &mut normal, &mut important, false);
    normal.0.extend(important.0);
    normal.0
}

/// Parse a declaration block, splitting declarations by the `!important`
/// marker into `(normal, important)` bags (the marker is stripped from the
/// values). Used by the stylesheet parser and the inline-`style` cascade pass.
pub(crate) fn parse_declaration_splits(block: &str) -> (CssAttr, CssAttr) {
    let mut input_buf = ParserInput::new(block);
    let mut input = CssParser::new(&mut input_buf);
    let mut normal = CssAttr::new();
    let mut important = CssAttr::new();
    parse_declarations_into(&mut input, &mut normal, &mut important, true);
    (normal, important)
}

/// Walk every `name: value [!important]` declaration in `input` and store it
/// in `normal` or `important` accordingly. Tolerates malformed input by
/// skipping to the next `;` rather than aborting the whole block.
///
/// When `expand_place` is set, the `place-*` box-alignment shorthands are
/// expanded into their longhands at parse time (browsers cascade shorthands
/// as expanded longhands, so the later declaration wins regardless of
/// shorthand / longhand form), and an invalid alignment value drops the
/// declaration instead of overriding an earlier expansion.
fn parse_declarations_into<'i, 't>(
    input: &mut CssParser<'i, 't>,
    normal: &mut CssAttr,
    important: &mut CssAttr,
    expand_place: bool,
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
        let is_important = input.try_parse(parse_important).is_ok();
        let cleaned = clean_value_text(value_text);
        if !name.is_empty() && !cleaned.is_empty() {
            let target = if is_important {
                &mut important.0
            } else {
                &mut normal.0
            };
            if expand_place && is_place_shorthand_name(&name) {
                // An invalid `place-*` value drops the whole declaration
                // (a browser parse error emits no longhands).
                if let Some(pairs) = expand_place_shorthand(&name, &cleaned) {
                    for (prop, value) in pairs {
                        target.insert(Arc::from(prop), value);
                    }
                }
            } else if expand_place
                && is_alignment_longhand(&name)
                && !alignment_longhand_valid(&name, &cleaned)
            {
                // An invalid alignment longhand is dropped rather than
                // stored, so it cannot override an earlier `place-*`
                // expansion through the per-property cascade.
            } else {
                target.insert(Arc::from(name.as_ref()), cleaned);
            }
        }
        // Consume the trailing `;` if present. A missing `;` is not an
        // error here — the parser simply stops at the next boundary.
        let _: Result<(), cssparser::BasicParseError<'_>> =
            input.try_parse(|i| i.expect_semicolon().map(|_| ()));
    }
}

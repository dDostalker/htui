use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, Delimiter, ParseError, Parser as CssParser,
    ParserInput, ParserState, QualifiedRuleParser, StyleSheetParser, parse_important,
};
use indexmap::IndexMap;
use log::{debug, warn};

use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use super::cascade::CascadeIndex;
use super::media::{MediaQueryList, parse_media_query_list};
use super::{
    properties::CssAttr,
    selector::CssSelector,
    values::{
        alignment_longhand_valid, expand_place_shorthand, is_alignment_longhand,
        is_list_style_type_keyword, is_place_shorthand_name,
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
    /// Enclosing `@media` condition (`None` = unconditional). The cascade
    /// skips the rule while the condition does not match the current
    /// terminal viewport.
    pub media: Option<MediaQueryList>,
}

/// The parsed author stylesheet: every qualified rule, with the `!important`
/// split already applied.
///
/// The second field is the lazily-built `CascadeIndex` — computed on the
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
    /// Create an empty rule set.
    pub fn new() -> Self {
        CssRule(Vec::new(), OnceLock::new())
    }

    /// The cascade index for this rule set, built on first use.
    pub(crate) fn cascade_index(&self) -> &CascadeIndex {
        self.1.get_or_init(|| CascadeIndex::build(self))
    }

    /// Append a rule with explicit normal / important declaration splits.
    pub fn push(&mut self, selector: CssSelector, normal: CssAttr, important: CssAttr) {
        self.push_media(selector, normal, important, None);
    }

    /// Append a rule with the declarations split above plus an enclosing
    /// `@media` condition.
    pub fn push_media(
        &mut self,
        selector: CssSelector,
        normal: CssAttr,
        important: CssAttr,
        media: Option<MediaQueryList>,
    ) {
        self.1.take();
        self.0.push(CssRuleEntry {
            selector,
            normal,
            important,
            media,
        });
    }

    /// Append a rule whose declarations all have normal priority.
    pub fn add(&mut self, selector: CssSelector, attr: CssAttr) {
        self.push(selector, attr, CssAttr::new());
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
    /// Create a stylesheet parser for `input`.
    pub fn new(input: &str) -> Self {
        Parser {
            context: input.to_string(),
        }
    }

    /// Parse the stylesheet and return the collected rules.
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
        for collected in state.rules.drain(..) {
            match collected {
                CollectedRule::Qualified(sel_text, normal, important) => {
                    // Split comma-separated selectors: `table, td { ... }` is
                    // two independent rules targeting distinct node types.
                    // Each sub-selector is parsed in isolation so it can stand
                    // on its own when applied via the cascade.
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
                // `@media` children were fully parsed (selector split
                // included) while their block was read; they carry their
                // condition on the entry.
                CollectedRule::Media(entry) => css.0.push(entry),
            }
        }
        debug!("CSS parsed: {} rules", css.0.len());
        css
    }
}

struct CssParseState {
    rules: Vec<CollectedRule>,
}
impl CssParseState {
    fn new() -> Self {
        Self { rules: vec![] }
    }
}

/// One collected top-level item, kept in source order: a plain qualified rule
/// (prelude text not yet selector-split) or a fully parsed `@media` child.
enum CollectedRule {
    Qualified(String, CssAttr, CssAttr),
    Media(CssRuleEntry),
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
        self.rules
            .push(CollectedRule::Qualified(prelude, normal, important));
        Ok(())
    }
}

/// Prelude collected by [`AtRuleParser`]: a parsed `@media` condition, or the
/// marker for at-rules the engine skips (`@font-face`, `@supports`, …).
enum AtRulePrelude {
    Media(MediaQueryList),
    Other,
}

impl<'i> AtRuleParser<'i> for CssParseState {
    type Prelude = AtRulePrelude;
    type AtRule = ();
    type Error = cssparser::BasicParseError<'i>;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<AtRulePrelude, ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("media") {
            let start = input.position();
            // `parse_until_before` skips balanced `(…)` groups as units, so
            // the slice below covers the whole prelude text including the
            // feature blocks (a plain `next()` loop would stop inside the
            // first `(` — block contents never advance the position).
            let _: std::result::Result<(), ParseError<()>> = input
                .parse_until_before(Delimiter::Semicolon | Delimiter::CurlyBracketBlock, |_| {
                    Ok(())
                });
            let text = input.slice_from(start);
            return match parse_media_query_list(text) {
                Some(list) => Ok(AtRulePrelude::Media(list)),
                None => Ok(AtRulePrelude::Other),
            };
        }
        debug!("skip @{}", name);
        while !input.is_exhausted() {
            input.next().ok();
        }
        Ok(AtRulePrelude::Other)
    }
    fn parse_block<'t>(
        &mut self,
        prelude: AtRulePrelude,
        _: &ParserState,
        input: &mut CssParser<'i, 't>,
    ) -> std::result::Result<(), ParseError<'i, Self::Error>> {
        let AtRulePrelude::Media(list) = prelude else {
            while !input.is_exhausted() {
                input.next().ok();
            }
            return Ok(());
        };
        // The block body is re-parsed through the full top-level parser, so
        // nested rules and declarations reuse the exact same code path.
        let body = serialize_rule_body(input);
        let mut sub = Parser::new(&body).parse();
        for mut entry in sub.0.drain(..) {
            if entry.media.is_some() {
                warn!("Nested @media is not supported — dropping rule");
                continue;
            }
            entry.media = Some(list.clone());
            self.rules.push(CollectedRule::Media(entry));
        }
        Ok(())
    }
    fn rule_without_block(
        &mut self,
        _: AtRulePrelude,
        _: &ParserState,
    ) -> std::result::Result<(), ()> {
        Ok(())
    }
}

/// Serialize the remaining tokens of `input` back to CSS text.
///
/// Used for `@media` block bodies. Balanced blocks (`{ … }`, `( … )`, function
/// calls) are pre-scanned by the tokenizer as single units whose contents
/// never advance the outer token stream, so a plain `slice_from` walk would
/// silently drop them — [`CssParser::parse_block`] enters the block and its
/// contents resume as the nested stream, which is serialized recursively.
/// Everything else is taken verbatim from the source.
fn serialize_rule_body<'i, 't>(input: &mut CssParser<'i, 't>) -> String {
    let mut out = String::new();
    serialize_tokens(input, &mut out);
    out
}

fn serialize_tokens<'i, 't>(input: &mut CssParser<'i, 't>, out: &mut String) {
    while !input.is_exhausted() {
        let start = input.position();
        let tok = match input.next_including_whitespace_and_comments() {
            Ok(t) => t,
            Err(_) => break,
        };
        let block: Option<(&str, &str)> = match tok {
            cssparser::Token::CurlyBracketBlock => Some(("{", "}")),
            cssparser::Token::ParenthesisBlock => Some(("(", ")")),
            cssparser::Token::SquareBracketBlock => Some(("[", "]")),
            // A function token's source slice already covers `name(`.
            cssparser::Token::Function(_) => Some((input.slice_from(start), ")")),
            _ => None,
        };
        match block {
            None => out.push_str(input.slice_from(start)),
            Some((open, close)) => {
                out.push_str(open);
                let _: std::result::Result<(), ParseError<()>> =
                    input.parse_nested_block(|nested| {
                        serialize_tokens(nested, out);
                        Ok(())
                    });
                out.push_str(close);
            }
        }
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
        // CSS property names are ASCII case-insensitive; normalise to
        // lowercase so `WIDTH: 10`, `BACKGROUND-COLOR: red` and `Text-Align`
        // all reference the same canonical keys the `CssAttr` accessors (and
        // the cascade's `SUPPORTED_PROPERTIES`) use.
        let name = name.to_ascii_lowercase();
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
            // Shorthands that expand at declaration-parse time: browsers
            // cascade shorthands as their expanded longhands (the later
            // declaration wins regardless of form), and an invalid value
            // drops the whole declaration instead of overriding an earlier
            // expansion through the raw-name slot.
            type Expansion = Vec<(&'static str, Arc<str>)>;
            let expansion: Option<Option<Expansion>> = if !expand_place {
                None
            } else if is_place_shorthand_name(&name) {
                Some(expand_place_shorthand(&name, &cleaned))
            } else if name == "list-style" {
                Some(expand_list_style_shorthand(&cleaned))
            } else if name == "font" {
                Some(expand_font_shorthand(&cleaned))
            } else {
                None
            };
            match expansion {
                Some(pairs) => {
                    for (prop, value) in pairs.unwrap_or_default() {
                        target.insert(Arc::from(prop), value);
                    }
                }
                None => {
                    if expand_place
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
            }
        }
        // Consume the trailing `;` if present. A missing `;` is not an
        // error here — the parser simply stops at the next boundary.
        let _: Result<(), cssparser::BasicParseError<'_>> =
            input.try_parse(|i| i.expect_semicolon().map(|_| ()));
    }
}

/// Expand a `list-style` shorthand declaration.
///
/// Only the `list-style-type` component has a terminal counterpart; the
/// `position` (`inside` / `outside`) and `image` components are accepted
/// (they make the declaration valid) but produce no longhand. An empty value
/// fails the whole declaration.
fn expand_list_style_shorthand(value: &str) -> Option<Vec<(&'static str, Arc<str>)>> {
    let words: Vec<&str> = value.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for tok in words {
        if is_list_style_type_keyword(tok) {
            out.push(("list-style-type", Arc::from(tok)));
        }
    }
    Some(out)
}

/// Expand a `font` shorthand declaration (`font: <style>? <weight>?
/// <size>[/<line-height>?] <family>`).
///
/// Terminal-relevant longhands only: `font-style` / `font-weight` /
/// `font-size` / `line-height`. The font family must be present (any
/// identifier — there are no font families in a terminal) and the size must
/// be numeric, mirroring the CSS grammar; `variant` flags like `small-caps`
/// and the system-font keywords (`caption`, `icon`, …) are not supported and
/// fail the declaration.
fn expand_font_shorthand(value: &str) -> Option<Vec<(&'static str, Arc<str>)>> {
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let mut style: Option<&str> = None;
    let mut weight: Option<&str> = None;
    let mut i = 0usize;
    // Leading flags: style / weight / variant, any order, each at most once
    // (a repeat is a grammar error in CSS; here the second occurrence just
    // stops the scan and then fails the size parse below).
    while let Some(&tok) = tokens.get(i) {
        match tok.to_ascii_lowercase().as_str() {
            "italic" | "oblique" => style = Some(tok),
            "bold" | "bolder" | "lighter" => weight = Some(tok),
            // `normal` / `small-caps` reset or set components we do not
            // track — accepted, produces nothing.
            "normal" | "small-caps" => {}
            _ => break,
        }
        i += 1;
    }
    // Size (with optional `/line-height` attached or space-separated).
    let size_tok = tokens.get(i)?;
    let (size, mut line_height) = match size_tok.split_once('/') {
        Some((s, lh)) => (s.to_string(), Some(lh.to_string())),
        None => (size_tok.to_string(), None),
    };
    let looks_numeric = size
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '.');
    if !looks_numeric {
        return None;
    }
    i += 1;
    if line_height.is_none()
        && let Some(&tok) = tokens.get(i)
        && let Some(lh) = tok.strip_prefix('/')
    {
        if lh.is_empty() {
            return None;
        }
        line_height = Some(lh.to_string());
        i += 1;
    }
    // The family list is mandatory in CSS (any identifier(s)).
    tokens.get(i)?;
    let mut out = Vec::new();
    if let Some(s) = style {
        out.push(("font-style", Arc::from(s)));
    }
    if let Some(w) = weight {
        out.push(("font-weight", Arc::from(w)));
    }
    out.push(("font-size", Arc::from(size)));
    if let Some(lh) = line_height {
        out.push(("line-height", Arc::from(lh)));
    }
    Some(out)
}

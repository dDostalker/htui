use std::str::FromStr;

use super::values::Viewport;
use crate::tdom::node::{NodeId, NodeType};

/// Errors produced while parsing a CSS selector string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectorError {
    /// The selector string is empty.
    #[error("empty selector")]
    EmptySelector,
    /// A compound part (e.g. `.class`, `#id`) is empty.
    #[error("empty compound")]
    EmptyCompound,
    /// A class name after `.` is empty.
    #[error("invalid class")]
    InvalidClass,
    /// An id after `#` is empty.
    #[error("invalid id")]
    InvalidId,
    /// A bare tag name is empty.
    #[error("invalid tag")]
    InvalidTag,
    /// `:hover` was given arguments, which it does not take.
    #[error(":hover takes no arguments")]
    HoverTakesNoArguments,
    /// `:focus` was given arguments, which it does not take.
    #[error(":focus takes no arguments")]
    FocusTakesNoArguments,
    /// `:nth-child` is missing its `(...)` argument list.
    #[error(":nth-child requires `(...)`")]
    NthChildRequiresParens,
    /// `:nth-child(...)` is missing the closing `)`.
    #[error(":nth-child(...) missing ')'")]
    NthChildMissingCloseParen,
    /// The `:nth-child` argument is not a supported `an+b` form.
    #[error("invalid :nth-child arg: {0:?}")]
    InvalidNthChildArg(String),
    /// `:nth-last-child` is missing its `(...)` argument list.
    #[error(":nth-last-child requires `(...)`")]
    NthLastChildRequiresParens,
    /// `:nth-last-child(...)` is missing the closing `)`.
    #[error(":nth-last-child(...) missing ')'")]
    NthLastChildMissingCloseParen,
    /// The `:nth-last-child` argument is not a supported `an+b` form.
    #[error("invalid :nth-last-child arg: {0:?}")]
    InvalidNthLastChildArg(String),
    /// `:not` is missing its `(...)` argument list.
    #[error(":not requires `(...)`")]
    NotRequiresParens,
    /// `:not(...)` is missing the closing `)`.
    #[error(":not(...) missing ')'")]
    NotMissingCloseParen,
    /// `:not(...)` arguments nest deeper than the supported limit.
    #[error(":not(...) nesting too deep")]
    NotNestedTooDeep,
    /// The pseudo-class is not one of the supported set.
    #[error("unsupported pseudo-class: :{0}")]
    UnsupportedPseudoClass(String),
    /// The tag name does not map to any known `NodeType`.
    #[error("unsupported CSS tag: {0:?}")]
    UnsupportedTag(String),
    /// `:nth-of-type` / `:nth-last-of-type` is missing its `(...)` list.
    #[error(":{0} requires `(...)`")]
    NthOfTypeRequiresParens(String),
    /// `:nth-of-type(...)` / `:nth-last-of-type(...)` is missing the `)`.
    #[error(":{0}(...) missing ')'")]
    NthOfTypeMissingCloseParen(String),
    /// The `:nth-of-type` / `:nth-last-of-type` argument is not a supported
    /// `an+b` form.
    #[error("invalid :{0} arg: {1:?}")]
    InvalidNthOfTypeArg(String, String),
}

/// `an + b` expression used by the `:nth-child()` pseudo-class.
///
/// The selector matches an element when its 1-based index among its parent's
/// siblings equals `a*n + b` for some non-negative integer `n`. Supported
/// forms are `even`, `odd`, and bare integers like `:nth-child(2)`.
/// More complex forms (`-n+3`, `n+2` with implicit coefficient) are rejected
/// during parsing so we never silently mis-match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NthExpr {
    /// Step. `0` means the expression degenerates to "match only index `b`".
    pub a: usize,
    /// 1-based offset (matches indices `b`, `a + b`, `2a + b`, …).
    pub b: usize,
}

impl NthExpr {
    /// `2n` — every 2nd child starting at the 2nd.
    pub const EVEN: NthExpr = NthExpr { a: 2, b: 0 };
    /// `2n + 1` — every 2nd child starting at the 1st.
    pub const ODD: NthExpr = NthExpr { a: 2, b: 1 };

    /// Evaluate the expression against a 1-based sibling index.
    pub fn matches(self, index_1based: usize) -> bool {
        if self.a == 0 {
            return index_1based == self.b;
        }
        index_1based >= self.b && (index_1based - self.b).is_multiple_of(self.a)
    }
}

/// Session-level dynamic pseudo-class state (`:hover` / `:focus`).
///
/// One slot per dynamic pseudo-class, populated by whoever owns that piece of
/// interaction state (the script thread's pointer state machine fills
/// `hovered`; the input loop's focus state fills `focused`). The cascade and
/// the render thread treat it as a single value: adding a pseudo-class means
/// adding one field here, one `PseudoClass` variant and one method arm.
/// Session-level dynamic pseudo-class state (`:hover` / `:focus`) plus the
/// cascade environment.
///
/// One slot per dynamic pseudo-class, populated by whoever owns that piece of
/// interaction state (the script thread's pointer state machine fills
/// `hovered`; the input loop's focus state fills `focused`). The cascade and
/// the render thread treat it as a single value: adding a pseudo-class means
/// adding one field here, one `PseudoClass` variant and one method arm.
///
/// `viewport` carries the terminal cell grid for `@media` matching. The
/// default (`0x0`) matches no size condition — unconditional rules are
/// unaffected — so callers that never cascade a real viewport keep working.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DynamicState {
    /// The node currently under the pointer (`:hover` matches it and its
    /// ancestors).
    pub hovered: Option<NodeId>,
    /// The focused text field (`:focus` matches it exactly).
    pub focused: Option<NodeId>,
    /// Terminal viewport (columns × rows in cells) for `@media` conditions.
    pub viewport: Viewport,
}

/// The set of dynamic pseudo-classes a stylesheet actually uses, packed as a
/// bitmask so rule detection is a single parse-time pass and command
/// processing is an O(1) membership test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PseudoClassSet(u8);

impl PseudoClassSet {
    /// No dynamic pseudo-class rules.
    pub const EMPTY: Self = Self(0);
    /// `:hover` rules present.
    pub const HOVER: Self = Self(1 << 0);
    /// `:focus` rules present.
    pub const FOCUS: Self = Self(1 << 1);

    /// Whether `pseudo` is a member of the set.
    pub fn contains_pseudo(&self, pseudo: PseudoClass) -> bool {
        self.0 & pseudo.as_flag().0 != 0
    }

    /// Union of two sets.
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The pseudo-classes appearing anywhere in a selector chain.
    pub fn from_selector(sel: &CssSelector) -> Self {
        let mut set = Self::EMPTY;
        for compound in &sel.compounds {
            for s in compound {
                if let Selector::PseudoClass(p) = s {
                    set = set.union(p.as_flag());
                }
            }
        }
        set
    }
}

/// Dynamic pseudo-classes supported by the selector parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoClass {
    /// `:hover` — matches while the pointer is over the element (or one of
    /// its descendants). Evaluated against the cascade's hovered node.
    Hover,
    /// `:focus` — matches the element that currently holds keyboard focus
    /// (the session's focused text field). Unlike `:hover`, it matches the
    /// focused element itself, not its ancestors (`:focus-within` would).
    Focus,
}

impl PseudoClass {
    /// The bit this pseudo-class maps to in [`PseudoClassSet`].
    pub fn as_flag(&self) -> PseudoClassSet {
        match self {
            PseudoClass::Hover => PseudoClassSet::HOVER,
            PseudoClass::Focus => PseudoClassSet::FOCUS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Selector {
    /// `*` — the universal selector: matches any element (never text,
    /// comment or document nodes). Contributes nothing to specificity.
    Universal,
    Tag(NodeType),
    Class(String),
    Id(String),
    /// `:nth-child(<expr>)` — match when the element is the n-th child of its
    /// parent (counting all siblings, including text nodes, per CSS spec).
    NthChild(NthExpr),
    /// `:nth-last-child(<expr>)` — the `:nth-child` mirror counting from the
    /// end of the sibling list (the last child is index 1).
    NthLastChild(NthExpr),
    /// `:nth-of-type(<expr>)` — like `:nth-child` but counting only siblings
    /// of the same element type (same `NodeType`; text / comment siblings
    /// never count, matching the CSS "of its type" semantics).
    NthOfType(NthExpr),
    /// `:nth-last-of-type(<expr>)` — the `:nth-of-type` mirror counting
    /// same-type siblings from the end.
    NthLastOfType(NthExpr),
    /// `:root` — the document's root element (the child of the `Document`
    /// node, i.e. `<html>` in a parsed document).
    Root,
    /// `:empty` — an element with no children that affect emptiness: element
    /// children always do, text children do when their data is non-empty
    /// (whitespace counts), comments never do (CSS Selectors semantics).
    Empty,
    /// `:not(<compound list>)` — matches when **none** of the argument
    /// compound selectors match. Arguments are comma-separated compounds
    /// without combinators (the common L3/L4 subset).
    Not(Vec<CompoundSelector>),
    /// `:checked` — a checked `<input type=checkbox|radio>` (the `checked`
    /// attribute the engine toggles on click) or a `<option selected>`.
    Checked,
    /// `:enabled` — a form control that is not `:disabled`.
    Enabled,
    /// `:disabled` — a form control carrying the `disabled` attribute, or a
    /// descendant of a `<fieldset disabled>` (legend exemption not modelled).
    Disabled,
    /// `:lang(<tag>)` — the element (or its nearest ancestor with a `lang`
    /// attribute) declares a language whose primary tag matches (case-
    /// insensitive; `zh` matches `lang="zh-CN"`).
    Lang(String),
    /// A dynamic pseudo-class (currently `:hover`).
    PseudoClass(PseudoClass),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

pub type CompoundSelector = Vec<Selector>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CssSelector {
    pub compounds: Vec<CompoundSelector>,
    pub combinators: Vec<Combinator>,
    /// A trailing `::before` / `::after` pseudo-element. The compounds match
    /// the origin element; the pseudo-element's declarations are stored
    /// key-prefixed by the cascade (`content::before` …) and turned into
    /// generated runs by the inline collector.
    pub pseudo_element: Option<PseudoElement>,
}

impl FromStr for CssSelector {
    type Err = SelectorError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(SelectorError::EmptySelector);
        }
        // Split the selector chain into segments and the combinator that
        // introduces each segment. Whitespace is a descendant combinator; `>`,
        // `+`, `~` are explicit combinators. Empty segments (e.g. from `>` at
        // the start) are dropped — they're a parse error handled by the empty
        // check at the bottom.
        let (segments, combinators) = split_by_combinators(s);
        if segments.is_empty() {
            return Err(SelectorError::EmptySelector);
        }
        // A trailing `::before` / `::after` pseudo-element (inside the last
        // segment) is lifted off the chain: the chain itself matches the
        // origin element and the pseudo-element rides along as a flag.
        // Only `::before` / `::after` are recognized, including their
        // single-colon CSS compatibility spellings; other `::` names remain
        // unsupported pseudo-class errors.
        let mut pseudo_element = None;
        let mut segments: Vec<String> = segments.iter().map(|s| s.to_string()).collect();
        if let Some(last) = segments.last_mut()
            && let Some((trimmed, element)) = strip_pseudo_element(last)
        {
            pseudo_element = Some(element);
            if trimmed.trim().is_empty() {
                // The pseudo-element was the entire segment — a bare
                // `::before` has no origin to match.
                return Err(SelectorError::UnsupportedPseudoClass(match element {
                    PseudoElement::Before => "::before".to_string(),
                    PseudoElement::After => "::after".to_string(),
                }));
            }
            *last = trimmed.trim().to_string();
        }
        let mut compounds = Vec::with_capacity(segments.len());
        for seg in &segments {
            // A segment is one compound selector whose parts all apply to the
            // same element. The split is parenthesis-aware so whitespace
            // inside a function argument (`:nth-child(2n + 1)`,
            // `:not(p, div)`) never splits the pseudo-class in half.
            let mut compound = CompoundSelector::new();
            for part in split_top_level_whitespace(seg) {
                compound.extend(parse_compound(part)?);
            }
            compounds.push(compound);
        }
        Ok(CssSelector {
            compounds,
            combinators,
            pseudo_element,
        })
    }
}

/// The pseudo-elements the engine generates content for (CSS
/// `::before` / `::after`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Before,
    After,
}

/// If `seg` ends with a `::before` / `::after` pseudo-element suffix, return
/// the segment with the suffix stripped plus the element kind.
fn strip_pseudo_element(seg: &str) -> Option<(String, PseudoElement)> {
    let lower = seg.to_ascii_lowercase();
    for (suffix, element) in [
        ("::before", PseudoElement::Before),
        (":before", PseudoElement::Before),
        ("::after", PseudoElement::After),
        (":after", PseudoElement::After),
    ] {
        if let Some(stripped) = lower.strip_suffix(suffix) {
            return Some((seg[..stripped.len()].to_string(), element));
        }
    }
    None
}

impl CssSelector {
    /// CSS specificity `(a, b, c)` of the selector chain, computed by summing
    /// every compound part:
    ///
    /// - `a` — id selectors (`#id`);
    /// - `b` — class selectors, attribute selectors and pseudo-classes
    ///   (`.class`, `:hover`, `:nth-child(...)`);
    /// - `c` — type selectors (`div`, `p`, …).
    ///
    /// Combinators contribute nothing. Used by the cascade to order rules so
    /// the most specific rule wins for a property regardless of source order.
    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut spec = (0u32, 0u32, 0u32);
        for compound in &self.compounds {
            let part = compound_specificity(compound);
            spec.0 += part.0;
            spec.1 += part.1;
            spec.2 += part.2;
        }
        spec
    }
}

/// Split a selector chain into segments and the combinator that introduces each
/// segment after the first.
///
/// `div p` → `(["div", "p"], [Descendant])`
/// `div > p` → `(["div", "p"], [Child])`
/// `div.foo` → `(["div.foo"], [])`
/// `div > p + span` → `(["div", "p", "span"], [Child, NextSibling])`
///
/// `(` and `[` open a balanced group; combinator chars inside a group
/// (e.g. the `+` in `:nth-child(3n+2)`) are kept as part of the segment
/// instead of being promoted to a sibling/child combinator.
fn split_by_combinators(s: &str) -> (Vec<&str>, Vec<Combinator>) {
    let mut segments: Vec<&str> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut iter = s.char_indices().peekable();
    let mut start = 0;
    let mut depth: usize = 0;

    while let Some((i, ch)) = iter.next() {
        // Track parentheses / brackets so combinator chars inside an
        // argument list (`:nth-child(3n+2)`, `:not(a > b)`) stay put.
        match ch {
            '(' | '[' => {
                depth = depth.saturating_add(1);
                continue;
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if depth > 0 {
            continue;
        }
        if !ch.is_whitespace() && !matches!(ch, '>' | '+' | '~') {
            continue;
        }
        let before = s[start..i].trim();
        if !before.is_empty() {
            segments.push(before);
        }
        // Skip whitespace surrounding the next segment, then look for an
        // explicit combinator char. An explicit `>`/`+`/`~` wins over a bare
        // whitespace (Descendant) combinator — so `div > p` is a single
        // `Child` combinator, not `Descendant + Child`.
        while let Some(&(_, next_ch)) = iter.peek() {
            if next_ch.is_whitespace() {
                iter.next();
            } else {
                break;
            }
        }
        let mut combinator = Combinator::Descendant;
        if let Some(&(_, next_ch)) = iter.peek() {
            combinator = match next_ch {
                '>' => {
                    iter.next();
                    Combinator::Child
                }
                '+' => {
                    iter.next();
                    Combinator::NextSibling
                }
                '~' => {
                    iter.next();
                    Combinator::SubsequentSibling
                }
                _ => Combinator::Descendant,
            };
            while let Some(&(_, next_ch)) = iter.peek() {
                if next_ch.is_whitespace() {
                    iter.next();
                } else {
                    break;
                }
            }
        }
        if iter.peek().is_some() {
            combinators.push(combinator);
        }
        start = iter.peek().map(|&(i, _)| i).unwrap_or(s.len());
    }

    let last = s[start..].trim();
    if !last.is_empty() {
        segments.push(last);
    }

    (segments, combinators)
}

fn parse_compound(s: &str) -> std::result::Result<CompoundSelector, SelectorError> {
    parse_compound_inner(s, 0)
}

/// Max supported `:not(...)` nesting depth. Real stylesheets never come close;
/// the cap stops pathological CSS from recursing the parser to stack
/// exhaustion.
const NOT_MAX_NESTING: usize = 8;

fn parse_compound_inner(
    s: &str,
    not_depth: usize,
) -> std::result::Result<CompoundSelector, SelectorError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(SelectorError::EmptyCompound);
    }
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' && chars[i] != ':' {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if name.is_empty() {
                    return Err(SelectorError::InvalidClass);
                }
                out.push(Selector::Class(name));
            }
            '#' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' && chars[i] != ':' {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if name.is_empty() {
                    return Err(SelectorError::InvalidId);
                }
                out.push(Selector::Id(name));
            }
            ':' => {
                // Pseudo-class. Pull the identifier, expect `(...)` where the
                // pseudo-class takes arguments, and emit the corresponding
                // Selector variant.
                i += 1;
                let name_start = i;
                while i < chars.len()
                    && chars[i] != '('
                    && chars[i] != '.'
                    && chars[i] != '#'
                    && chars[i] != ':'
                {
                    i += 1;
                }
                let name: String = chars[name_start..i].iter().collect();
                match name.to_ascii_lowercase().as_str() {
                    "hover" => {
                        if i < chars.len() && chars[i] == '(' {
                            return Err(SelectorError::HoverTakesNoArguments);
                        }
                        out.push(Selector::PseudoClass(PseudoClass::Hover));
                    }
                    "focus" => {
                        if i < chars.len() && chars[i] == '(' {
                            return Err(SelectorError::FocusTakesNoArguments);
                        }
                        out.push(Selector::PseudoClass(PseudoClass::Focus));
                    }
                    "nth-child" | "nth-last-child" => {
                        let from_last = name.eq_ignore_ascii_case("nth-last-child");
                        if i >= chars.len() || chars[i] != '(' {
                            return Err(if from_last {
                                SelectorError::NthLastChildRequiresParens
                            } else {
                                SelectorError::NthChildRequiresParens
                            });
                        }
                        let Some(arg) = capture_balanced_arg(&chars, &mut i) else {
                            return Err(if from_last {
                                SelectorError::NthLastChildMissingCloseParen
                            } else {
                                SelectorError::NthChildMissingCloseParen
                            });
                        };
                        let expr = parse_nth_child_arg(&arg).ok_or_else(|| {
                            if from_last {
                                SelectorError::InvalidNthLastChildArg(arg.clone())
                            } else {
                                SelectorError::InvalidNthChildArg(arg.clone())
                            }
                        })?;
                        out.push(if from_last {
                            Selector::NthLastChild(expr)
                        } else {
                            Selector::NthChild(expr)
                        });
                    }
                    "nth-of-type" | "nth-last-of-type" => {
                        let lower_name = name.to_ascii_lowercase();
                        if i >= chars.len() || chars[i] != '(' {
                            return Err(SelectorError::NthOfTypeRequiresParens(lower_name));
                        }
                        let Some(arg) = capture_balanced_arg(&chars, &mut i) else {
                            return Err(SelectorError::NthOfTypeMissingCloseParen(lower_name));
                        };
                        let from_last = lower_name == "nth-last-of-type";
                        let expr = parse_nth_child_arg(&arg).ok_or_else(|| {
                            SelectorError::InvalidNthOfTypeArg(lower_name, arg.clone())
                        })?;
                        out.push(if from_last {
                            Selector::NthLastOfType(expr)
                        } else {
                            Selector::NthOfType(expr)
                        });
                    }
                    // `:first-child` / `:last-child` / `:only-child` are
                    // positional special cases of the nth machinery: they
                    // decompose into index-1-from-start / index-1-from-end
                    // expressions, so the matcher keeps one code path. The
                    // `*-of-type` forms do the same over the same-type index.
                    "first-child" => out.push(Selector::NthChild(FIRST_CHILD_EXPR)),
                    "last-child" => out.push(Selector::NthLastChild(FIRST_CHILD_EXPR)),
                    "only-child" => {
                        out.push(Selector::NthChild(FIRST_CHILD_EXPR));
                        out.push(Selector::NthLastChild(FIRST_CHILD_EXPR));
                    }
                    "first-of-type" => out.push(Selector::NthOfType(FIRST_CHILD_EXPR)),
                    "last-of-type" => out.push(Selector::NthLastOfType(FIRST_CHILD_EXPR)),
                    "only-of-type" => {
                        out.push(Selector::NthOfType(FIRST_CHILD_EXPR));
                        out.push(Selector::NthLastOfType(FIRST_CHILD_EXPR));
                    }
                    "root" => out.push(Selector::Root),
                    "empty" => out.push(Selector::Empty),
                    "checked" => out.push(Selector::Checked),
                    "enabled" => out.push(Selector::Enabled),
                    "disabled" => out.push(Selector::Disabled),
                    "lang" => {
                        if i >= chars.len() || chars[i] != '(' {
                            return Err(SelectorError::NthOfTypeRequiresParens("lang".to_string()));
                        }
                        let Some(arg) = capture_balanced_arg(&chars, &mut i) else {
                            return Err(SelectorError::NthOfTypeMissingCloseParen(
                                "lang".to_string(),
                            ));
                        };
                        let tag = arg.trim().to_ascii_lowercase();
                        if tag.is_empty() || tag.contains(char::is_whitespace) {
                            return Err(SelectorError::InvalidNthOfTypeArg(
                                "lang".to_string(),
                                arg.clone(),
                            ));
                        }
                        out.push(Selector::Lang(tag));
                    }
                    "not" => {
                        if i >= chars.len() || chars[i] != '(' {
                            return Err(SelectorError::NotRequiresParens);
                        }
                        let Some(arg) = capture_balanced_arg(&chars, &mut i) else {
                            return Err(SelectorError::NotMissingCloseParen);
                        };
                        if not_depth >= NOT_MAX_NESTING {
                            return Err(SelectorError::NotNestedTooDeep);
                        }
                        out.push(Selector::Not(parse_not_args(&arg, not_depth + 1)?));
                    }
                    _ => return Err(SelectorError::UnsupportedPseudoClass(name)),
                }
            }
            _ => {
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' && chars[i] != ':' {
                    i += 1;
                }
                let tag: String = chars[start..i].iter().collect();
                if tag.is_empty() {
                    return Err(SelectorError::InvalidTag);
                }
                if tag == "*" {
                    out.push(Selector::Universal);
                } else {
                    out.push(Selector::Tag(tag_str_to_node_type(&tag)?));
                }
            }
        }
    }
    Ok(out)
}

/// Parse the argument of `:nth-child(...)`.
///
/// Accepted `an+b` forms:
/// - `even`     → `2n`     (every 2nd starting at 2)
/// - `odd`      → `2n + 1` (every 2nd starting at 1)
/// - `<n>`      → bare integer, matches exactly that 1-based index
/// - `<a>n`     → e.g. `3n` matches the 3rd, 6th, 9th, …
/// - `<a>n+<b>` / `<a>n-<b>` → e.g. `2n+1` matches the 1st, 3rd, 5th, …
///
/// Returns `None` for unsupported forms (`-n+3`, `n+2` with implicit `1n`,
/// `n` alone, signed/zero coefficients, trailing junk) so the parser can
/// surface them as errors instead of silently mis-matching.
fn parse_nth_child_arg(s: &str) -> Option<NthExpr> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower == "even" {
        return Some(NthExpr::EVEN);
    }
    if lower == "odd" {
        return Some(NthExpr::ODD);
    }
    // Bare integer → fixed position.
    if let Ok(n) = s.parse::<usize>() {
        if n == 0 {
            return None;
        }
        return Some(NthExpr { a: 0, b: n });
    }
    // Otherwise expect `<a>n[+|-<b>]`. Reject sign on `a`, implicit `1n`,
    // signed offsets, and trailing whitespace/junk.
    let (coef_part, tail) = {
        let idx = lower.find('n')?;
        lower.split_at(idx)
    };

    let tail = &tail[1..]; // skip the 'n' itself
    let a: usize = if coef_part.is_empty() {
        return None; // bare `n` is not supported
    } else {
        coef_part.parse().ok()?
    };
    if a == 0 {
        return None;
    }
    let tail = tail.trim();
    if tail.is_empty() {
        return Some(NthExpr { a, b: 0 });
    }
    let (sign, num) = match tail.chars().next()? {
        '+' => (1i32, &tail[1..]),
        '-' => (-1i32, &tail[1..]),
        _ => return None,
    };
    let n: i32 = num.trim().parse().ok()?;
    if n == 0 {
        return None;
    }
    let b_signed = n * sign;
    if b_signed <= 0 {
        return None;
    }
    Some(NthExpr {
        a,
        b: b_signed as usize,
    })
}

/// Specificity of one compound selector (the per-compound slice of
/// [`CssSelector::specificity`]): ids in `a`, classes / pseudo-classes in
/// `b`, tags in `c`. Used by `:not(...)` which takes its most specific
/// argument's specificity.
fn compound_specificity(compound: &[Selector]) -> (u32, u32, u32) {
    let mut spec = (0u32, 0u32, 0u32);
    for sel in compound {
        match sel {
            Selector::Universal => {}
            Selector::Id(_) => spec.0 += 1,
            Selector::Class(_)
            | Selector::PseudoClass(_)
            | Selector::NthChild(_)
            | Selector::NthLastChild(_)
            | Selector::NthOfType(_)
            | Selector::NthLastOfType(_)
            | Selector::Root
            | Selector::Empty
            | Selector::Checked
            | Selector::Enabled
            | Selector::Disabled
            | Selector::Lang(_) => spec.1 += 1,
            Selector::Not(compounds) => {
                let arg = compounds
                    .iter()
                    .map(|c| compound_specificity(c))
                    .max()
                    .unwrap_or((0, 0, 0));
                spec.0 += arg.0;
                spec.1 += arg.1;
                spec.2 += arg.2;
            }
            Selector::Tag(_) => spec.2 += 1,
        }
    }
    spec
}

fn tag_str_to_node_type(tag: &str) -> std::result::Result<NodeType, SelectorError> {
    // Reverse lookup against the single tag table on [`NodeType`] — see
    // [`NodeType::from_tag_name`]. Keeps the selector mapping in sync with
    // the HTML parser and the DOM's `tag_name` by construction.
    NodeType::from_tag_name(tag).ok_or_else(|| SelectorError::UnsupportedTag(tag.to_string()))
}

/// `:first-child` / `:only-child` decompose to "index 1 from the start" —
/// the same `NthExpr` the nth machinery already evaluates.
const FIRST_CHILD_EXPR: NthExpr = NthExpr { a: 0, b: 1 };

/// Capture the balanced `(...)` argument whose `(` is at `chars[*i]`,
/// advancing `*i` past the closing `)`. Unlike the plain scan, nested
/// parentheses (e.g. the inner `:nth-child(2)` of `:not(:nth-child(2))`)
/// stay inside the argument. Returns `None` when the `)` never comes.
fn capture_balanced_arg(chars: &[char], i: &mut usize) -> Option<String> {
    let arg_start = *i + 1;
    let mut depth = 0usize;
    while *i < chars.len() {
        match chars[*i] {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let arg: String = chars[arg_start..*i].iter().collect();
                    *i += 1;
                    return Some(arg);
                }
            }
            _ => {}
        }
        *i += 1;
    }
    None
}

/// Parse the argument of `:not(...)`: a comma-separated list of compound
/// selectors without combinators (the common L3/L4 subset). Combinators
/// inside an argument fail with the argument's own parse error.
fn parse_not_args(
    arg: &str,
    not_depth: usize,
) -> std::result::Result<Vec<CompoundSelector>, SelectorError> {
    let parts = split_top_level_commas(arg);
    if parts.is_empty() {
        return Err(SelectorError::EmptyCompound);
    }
    parts
        .into_iter()
        .map(|part| parse_compound_inner(part, not_depth))
        .collect()
}

/// Split on whitespace that sits outside `(...)` / `[...]`, so the space
/// inside a function argument (`:nth-child(2n + 1)`) never splits the
/// pseudo-class in half.
fn split_top_level_whitespace(s: &str) -> Vec<&str> {
    split_top_level(s, char::is_whitespace)
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect()
}

/// Split on top-level commas (outside `(...)` / `[...]`), trimming each part
/// and dropping empties: the `:not( p , div )` argument yields `["p", "div"]`.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    split_top_level(s, |c| c == ',')
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

/// Shared splitter for `split_top_level_whitespace` / `split_top_level_commas`:
/// cut at every `is_sep` character not enclosed in parentheses or brackets.
fn split_top_level(s: &str, is_sep: impl Fn(char) -> bool) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' | '[' => depth = depth.saturating_add(1),
            ')' | ']' => depth = depth.saturating_sub(1),
            c if depth == 0 && is_sep(c) => {
                parts.push(&s[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

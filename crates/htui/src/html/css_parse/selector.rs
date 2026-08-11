use std::str::FromStr;

use crate::tdom::node::NodeType;

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
    /// `:nth-child` is missing its `(...)` argument list.
    #[error(":nth-child requires `(...)`")]
    NthChildRequiresParens,
    /// `:nth-child(...)` is missing the closing `)`.
    #[error(":nth-child(...) missing ')'")]
    NthChildMissingCloseParen,
    /// The `:nth-child` argument is not a supported `an+b` form.
    #[error("invalid :nth-child arg: {0:?}")]
    InvalidNthChildArg(String),
    /// The pseudo-class is not one of the supported set.
    #[error("unsupported pseudo-class: :{0}")]
    UnsupportedPseudoClass(String),
    /// The tag name does not map to any known `NodeType`.
    #[error("unsupported CSS tag: {0:?}")]
    UnsupportedTag(String),
}

/// `an + b` expression used by the `:nth-child()` pseudo-class.
///
/// The selector matches an element when its 1-based index among its parent's
/// siblings equals `a*n + b` for some non-negative integer `n`. This is the
/// minimal subset of the CSS formula that the existing visual tests need
/// (`even` → `2n`, `odd` → `2n+1`, and bare integers like `:nth-child(2)`).
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

/// Dynamic pseudo-classes supported by the selector parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoClass {
    /// `:hover` — matches while the pointer is over the element (or one of
    /// its descendants). Evaluated against the cascade's hovered node.
    Hover,
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
        let mut compounds = Vec::with_capacity(segments.len());
        for seg in &segments {
            // A segment is one compound selector whose parts all apply to the
            // same element. Whitespace within a segment is impossible because
            // `split_by_combinators` already consumed it as a divider, but we
            // keep the inner split for safety against future callers.
            let mut compound = CompoundSelector::new();
            for part in seg.split_whitespace().filter(|p| !p.is_empty()) {
                compound.extend(parse_compound(part)?);
            }
            compounds.push(compound);
        }
        Ok(CssSelector {
            compounds,
            combinators,
        })
    }
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
            for sel in compound {
                match sel {
                    // The universal selector contributes nothing to
                    // specificity (CSS spec: `*` is worth `(0,0,0)`).
                    Selector::Universal => {}
                    Selector::Id(_) => spec.0 += 1,
                    Selector::Class(_) | Selector::PseudoClass(_) | Selector::NthChild(_) => {
                        spec.1 += 1
                    }
                    Selector::Tag(_) => spec.2 += 1,
                }
            }
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
                // Pseudo-class (currently only `:nth-child(<expr>)` is
                // recognised). Pull the identifier, expect `(...)`, parse
                // the argument, and emit the corresponding Selector variant.
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
                if name.eq_ignore_ascii_case("hover") {
                    if i < chars.len() && chars[i] == '(' {
                        return Err(SelectorError::HoverTakesNoArguments);
                    }
                    out.push(Selector::PseudoClass(PseudoClass::Hover));
                } else if name.eq_ignore_ascii_case("nth-child") {
                    if i >= chars.len() || chars[i] != '(' {
                        return Err(SelectorError::NthChildRequiresParens);
                    }
                    i += 1;
                    let arg_start = i;
                    while i < chars.len() && chars[i] != ')' {
                        i += 1;
                    }
                    if i >= chars.len() {
                        return Err(SelectorError::NthChildMissingCloseParen);
                    }
                    let arg: String = chars[arg_start..i].iter().collect();
                    i += 1; // consume ')'
                    let expr = parse_nth_child_arg(&arg)
                        .ok_or_else(|| SelectorError::InvalidNthChildArg(arg.clone()))?;
                    out.push(Selector::NthChild(expr));
                } else {
                    return Err(SelectorError::UnsupportedPseudoClass(name));
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
/// Accepted forms (matching the subset of the CSS `an+b` grammar that the
/// visual tests use):
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

fn tag_str_to_node_type(tag: &str) -> std::result::Result<NodeType, SelectorError> {
    // Reverse lookup against the single tag table on [`NodeType`] — see
    // [`NodeType::from_tag_name`]. Keeps the selector mapping in sync with
    // the HTML parser and the DOM's `tag_name` by construction.
    NodeType::from_tag_name(tag).ok_or_else(|| SelectorError::UnsupportedTag(tag.to_string()))
}

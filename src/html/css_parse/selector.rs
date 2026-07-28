use std::str::FromStr;

use crate::tdom::node::NodeType;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Selector {
    Tag(NodeType),
    Class(String),
    Id(String),
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
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(anyhow::anyhow!("empty selector"));
        }
        // Split the selector chain into segments and the combinator that
        // introduces each segment. Whitespace is a descendant combinator; `>`,
        // `+`, `~` are explicit combinators. Empty segments (e.g. from `>` at
        // the start) are dropped — they're a parse error handled by the empty
        // check at the bottom.
        let (segments, combinators) = split_by_combinators(s);
        if segments.is_empty() {
            return Err(anyhow::anyhow!("empty selector"));
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

/// Split a selector chain into segments and the combinator that introduces each
/// segment after the first.
///
/// `div p` → `(["div", "p"], [Descendant])`
/// `div > p` → `(["div", "p"], [Child])`
/// `div.foo` → `(["div.foo"], [])`
/// `div > p + span` → `(["div", "p", "span"], [Child, NextSibling])`
fn split_by_combinators(s: &str) -> (Vec<&str>, Vec<Combinator>) {
    let mut segments: Vec<&str> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut iter = s.char_indices().peekable();
    let mut start = 0;

    while let Some((i, ch)) = iter.next() {
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

fn parse_compound(s: &str) -> std::result::Result<CompoundSelector, anyhow::Error> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow::anyhow!("empty compound"));
    }
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if name.is_empty() {
                    return Err(anyhow::anyhow!("invalid class"));
                }
                out.push(Selector::Class(name));
            }
            '#' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if name.is_empty() {
                    return Err(anyhow::anyhow!("invalid id"));
                }
                out.push(Selector::Id(name));
            }
            _ => {
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '#' {
                    i += 1;
                }
                let tag: String = chars[start..i].iter().collect();
                if tag.is_empty() {
                    return Err(anyhow::anyhow!("invalid tag"));
                }
                out.push(Selector::Tag(tag_str_to_node_type(&tag)?));
            }
        }
    }
    Ok(out)
}

fn tag_str_to_node_type(tag: &str) -> std::result::Result<NodeType, anyhow::Error> {
    let t = tag.to_ascii_lowercase();
    match t.as_str() {
        "a" => Ok(NodeType::A),
        "article" => Ok(NodeType::Article),
        "aside" => Ok(NodeType::Aside),
        "body" => Ok(NodeType::Body),
        "button" => Ok(NodeType::Button),
        "details" => Ok(NodeType::Details),
        "div" => Ok(NodeType::Div),
        "figcaption" => Ok(NodeType::Figcaption),
        "figure" => Ok(NodeType::Figure),
        "footer" => Ok(NodeType::Footer),
        "form" => Ok(NodeType::Form),
        "header" => Ok(NodeType::Header),
        "h1" => Ok(NodeType::H1),
        "h2" => Ok(NodeType::H2),
        "h3" => Ok(NodeType::H3),
        "h4" => Ok(NodeType::H4),
        "h5" => Ok(NodeType::H5),
        "head" => Ok(NodeType::Head),
        "option" => Ok(NodeType::HtmlOption),
        "html" => Ok(NodeType::Html),
        "img" => Ok(NodeType::Img),
        "input" => Ok(NodeType::Input),
        "label" => Ok(NodeType::Label),
        "li" => Ok(NodeType::Li),
        "link" => Ok(NodeType::Link),
        "main" => Ok(NodeType::Main),
        "meta" => Ok(NodeType::Meta),
        "nav" => Ok(NodeType::Nav),
        "ol" => Ok(NodeType::Ol),
        "p" => Ok(NodeType::P),
        "section" => Ok(NodeType::Section),
        "select" => Ok(NodeType::Select),
        "span" => Ok(NodeType::Span),
        "style" => Ok(NodeType::Style),
        "summary" => Ok(NodeType::Summary),
        "textarea" => Ok(NodeType::Textarea),
        "title" => Ok(NodeType::Title),
        "ul" => Ok(NodeType::Ul),
        _ => Err(anyhow::anyhow!("unsupported CSS tag: {:?}", tag)),
    }
}

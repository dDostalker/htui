//! Outer `display` role of a box.
//!
//! Mirrors the CSS Display 3 model: a box's outer role is one of
//! *block-level* (the box participates in its parent's flow as a block)
//! or *inline-level* (it participates as an inline). Within each axis the
//! `container` / `block` flag distinguishes a *block container* (a box
//! that establishes a new formatting context for its children —
//! `display: block`, `flex`, `grid`, `table`, `flow-root`, etc.) from a
//! plain block-level box, and `inline-block` / `inline-flex` /
//! `inline-grid` / `inline-table` (inline-level boxes that establish
//! their own FC) from a plain `inline`.
//!
//! See [`OuterDisplay::from_css_display`] for the per-keyword mapping.

use crate::html::css_parse::CssDisplay;

/// Outer display role of a layout box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OuterDisplay {
    /// Block-level box. `container: true` means the box also
    /// establishes a new formatting context for its children (a
    /// *block container*). `container: false` is reserved for plain
    /// block-level boxes that don't establish their own FC; the v1
    /// renderer treats both cases identically.
    BlockLevel { container: bool },
    /// Inline-level box. `block: true` means the box establishes its
    /// own block formatting context (i.e. `inline-block` /
    /// `inline-flex` / `inline-grid` / `inline-table`).
    Inline { block: bool },
}

impl Default for OuterDisplay {
    /// Default outer role is a block container — matches the browser
    /// default for elements without an explicit `display`.
    fn default() -> Self {
        OuterDisplay::BlockLevel { container: true }
    }
}

impl OuterDisplay {
    /// Derive the outer role for a [`CssDisplay`](crate::html::css_parse::CssDisplay)
    /// value. See the variant docs for the per-keyword rule.
    pub fn from_css_display(display: CssDisplay) -> Self {
        match display {
            CssDisplay::Inline => OuterDisplay::Inline { block: false },
            CssDisplay::InlineBlock
            | CssDisplay::InlineFlex
            | CssDisplay::InlineGrid
            | CssDisplay::InlineTable => OuterDisplay::Inline { block: true },
            // `none` / `contents` boxes are dropped at generation time;
            // they should never reach here, but a `BlockLevel` default
            // is the safest fallback if a caller ever inspects them.
            CssDisplay::None | CssDisplay::Contents => {
                OuterDisplay::BlockLevel { container: false }
            }
            // All block-level keywords establish a new FC (or, for the
            // reserved grid variants, would). `flow-root` is the
            // canonical BFC root; the rest of the block-level set is
            // treated the same way for v1.
            _ => OuterDisplay::BlockLevel { container: true },
        }
    }

    /// `true` for inline-level boxes (`Inline` and `inline-block`-family).
    pub fn is_inline(self) -> bool {
        matches!(self, OuterDisplay::Inline { .. })
    }

    /// `true` for block-level boxes (the default).
    pub fn is_block_level(self) -> bool {
        matches!(self, OuterDisplay::BlockLevel { .. })
    }

    /// `true` for boxes that establish a new formatting context for
    /// their children (block container or inline-block).
    pub fn establishes_formatting_context(self) -> bool {
        matches!(self, OuterDisplay::BlockLevel { container: true } | OuterDisplay::Inline { block: true })
    }
}

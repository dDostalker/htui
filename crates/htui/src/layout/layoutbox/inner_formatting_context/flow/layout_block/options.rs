//! Per-element differences layered on top of the shared block-layout pipeline.
//!
//! Each block-level element has its own quirks: `<h1>` needs a `# ` prefix,
//! `<aside>` / `<header>` / `<footer>` paint with chrome regardless of
//! background, `<li>` indents by nesting level and offsets its children's
//! containing block past the marker width, etc. All of those go through
//! [`LayoutBlockOptions`]. The default value is "no special behaviour", so
//! the common case is `LayoutBlockOptions::default()`.

use crate::layout::InlineRun;

/// Per-element differences layered on top of the shared block-layout pipeline.
///
/// All fields default to "no special behaviour" so the common case is just
/// `LayoutBlockOptions::default()`.
#[derive(Debug, Default, Clone)]
pub struct LayoutBlockOptions {
    /// Inline run prepended to the body runs (e.g. `# ` for `<h1>`, `• ` for
    /// `<li>`). The run uses the element's own fg / bg / font style so the
    /// body runs can layer additional styles on top.
    pub prefix: Option<InlineRun>,
    /// When `true`, the rect's background is filled even without an explicit
    /// CSS `background-color` — used by `<aside>` (solid stripe) and
    /// `<header>` / `<footer>` (chrome bands).
    pub fill_bg_default: bool,
    /// Trailing blank rows reserved below the rect (default 0). The paint
    /// phase skips these rows when filling the background. `<header>` /
    /// `<footer>` use 2 to leave breathing room before the next block.
    pub spacing_after: usize,
    /// Extra columns added to `containing.x` and subtracted from
    /// `containing.width` when computing the border-box rect (used by nested
    /// `<ul>` / `<ol>` to indent items per nesting level).
    pub indent: usize,
    /// `(x_offset, width_delta)` applied to the rect's content-box to
    /// derive the children's containing block. `<li>` uses this to shift
    /// nested children past the marker width — `width_delta` is negative
    /// so the inner width shrinks by the same amount.
    pub child_containing_shift: Option<(usize, isize)>,
    /// Marks the block as a chrome element (`<aside>`/`<header>`/`<footer>`)
    /// whose background always covers the full border-box regardless of
    /// inline content width or text-align. Also covers blocks that opted
    /// into the same semantics via `fill_bg_default` (currently just the
    /// chrome trio).
    pub chrome: bool,
    /// Applies the browser user-agent `font-style: italic` default when no
    /// rule (author CSS or inherited value) has set `font-style` on the
    /// element — used by `<address>` the same way `<em>` does inline.
    pub ua_italic: bool,
    /// When `true`, the body runs are replaced with a single synthetic run of
    /// `─` glyphs spanning the content-box width — used by `<hr>` to paint a
    /// horizontal rule (the element has no inline content of its own).
    pub hr_rule: bool,
}

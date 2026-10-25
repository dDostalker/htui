use crate::html::css_parse::{CssDisplay, WhiteSpace};

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub enum NodeType {
    /// Document root (`nodeType` 9). The synthetic root node inserted by the
    /// HTML parser; never rendered directly.
    Document,
    /// `<!DOCTYPE html>` declaration (`nodeType` 10). The doctype name lives
    /// in [`Node::data`](crate::tdom::node::Node::data); never rendered.
    DocumentType,
    /// Document fragment (`nodeType` 11). Created via
    /// `document.createDocumentFragment()`; renders nothing until its
    /// children are appended elsewhere.
    DocumentFragment,
    /// Text node (`nodeType` 3). Content lives in [`Node::data`](crate::tdom::node::Node::data).
    Text,
    /// Comment node (`nodeType` 8). Content lives in [`Node::data`](crate::tdom::node::Node::data); never
    /// rendered.
    Comment,
    Span,
    Div,
    P,
    /// `<hr>` — thematic break / horizontal rule. Block element that paints
    /// a single row of `─` glyphs spanning the content-box width.
    Hr,
    Pre,
    Img,
    A,
    /// `<em>` — emphasis. Inline element with browser-default `font-style:
    /// italic` that the layout pass applies when neither the element nor an
    /// ancestor has declared an explicit `font-style`.
    Em,
    /// `<strong>` — strong importance. Inline element with browser-default
    /// `font-weight: bold`.
    Strong,
    /// `<b>` — attention without extra importance. Inline element with
    /// browser-default `font-weight: bold`.
    B,
    /// `<i>` — idiomatic text / voice. Inline element with browser-default
    /// `font-style: italic`.
    I,
    /// `<u>` — unarticulated annotation. Inline element with browser-default
    /// `text-decoration: underline`.
    U,
    /// `<s>` — no-longer-accurate text. Inline element with browser-default
    /// `text-decoration: line-through`.
    S,
    /// `<del>` — removed content (edits). Inline element with browser-default
    /// `text-decoration: line-through`.
    Del,
    /// `<ins>` — inserted content (edits). Inline element with browser-default
    /// `text-decoration: underline`.
    Ins,
    /// `<mark>` — highlighted reference. Inline element with browser-default
    /// yellow background and black foreground (applied when neither property
    /// has been declared on the element).
    Mark,
    /// `<q>` — short inline quotation. Inline element; the browser UA
    /// `::before`/`::after` quote marks are synthesised around the content
    /// (alternating double/single quotes by nesting depth).
    Q,
    /// `<abbr>` — abbreviation. Inline element; the browser UA `abbr[title]`
    /// dotted underline is approximated with a solid underline when a `title`
    /// attribute is present.
    Abbr,
    /// `<cite>` — cited work title. Inline element with browser-default
    /// `font-style: italic`.
    Cite,
    /// `<dfn>` — defining instance. Inline element with browser-default
    /// `font-style: italic`.
    Dfn,
    /// `<var>` — variable. Inline element with browser-default
    /// `font-style: italic`.
    Var,
    /// `<code>` — computer code. Inline element with browser-default
    /// monospace font (the terminal's native font, so no extra styling).
    Code,
    /// `<kbd>` — keyboard input. Inline element with browser-default
    /// monospace font.
    Kbd,
    /// `<samp>` — sample output. Inline element with browser-default
    /// monospace font.
    Samp,
    /// `<data>` — machine-readable value. Inline element with no UA visual
    /// default; the `value` attribute is machine-readable only and the text
    /// content renders as-is.
    Data,
    /// `<time>` — date/time value. Inline element with no UA visual default;
    /// the `datetime` attribute is machine-readable only and the text
    /// content renders as-is.
    Time,
    /// `<br>` — forced line break. Void element; the layout pass emits an
    /// unconditional break run that splits the visual line in every
    /// `white-space` mode.
    Br,
    Html,
    Body,
    Head,
    Title,
    Meta,
    /// `<base>` — document base URL. The element is never rendered; its
    /// `href` (see [`document_base_href`](crate::tdom::node::document_base_href)) establishes the base URL against
    /// which relative resource references (`<script src>`, `<link href>`,
    /// `<img src>`, JS `fetch`) resolve.
    Base,
    H1,
    H2,
    H3,
    H4,
    H5,
    /// `<h6>` — the smallest heading level. Block element with browser
    /// default `font-size: 0.67em` and a `###### ` markdown prefix, rendered
    /// through the same heading pipeline as `<h1>`–`<h5>`.
    H6,
    /// `<hgroup>` — a heading group (`<h1>`–`<h6>` members, possibly with a
    /// single `<p>`). Rendered as a plain block container; the members keep
    /// their own heading styling.
    HGroup,
    /// `<address>` — contact information for the nearest article or body
    /// ancestor. Block element with browser default `font-style: italic`
    /// that the layout pass applies when neither the element nor an ancestor
    /// has declared an explicit `font-style`.
    Address,
    /// `<blockquote>` — extended quotation. Block element with browser UA
    /// default margins (`margin-block: 1em`, `margin-inline: 40px`, adapted
    /// to 1 row / 4 terminal columns) that are dropped on any side the
    /// author overrides with an explicit `margin` declaration.
    BlockQuote,
    Header,
    Footer,
    Nav,
    Main,
    Section,
    Article,
    Aside,
    /// `<search>` — search / filter landmark container. Plain block element.
    Search,
    Figure,
    Figcaption,
    Details,
    Summary,
    /// `<dialog>` — a dialog box. Without the `open` attribute the browser UA
    /// stylesheet hides it (`dialog:not([open]) { display: none }`); with
    /// `open` it renders as a block with the UA default border. A submit
    /// button in a `<form method="dialog">` inside it natively closes it
    /// (removes `open`) and fires a `close` event.
    Dialog,
    Ul,
    Ol,
    Li,
    /// `<menu>` — command / menu list. Rendered like `<ul>`: each `<li>`
    /// child gets the `• ` marker and nesting indents per level.
    Menu,
    /// `<dl>` — description list container. Block element with browser UA
    /// default `margin-block: 1em` (one blank row above/below).
    Dl,
    /// `<dt>` — a term in a description list. Plain block element.
    Dt,
    /// `<dd>` — a description in a description list. Block element with
    /// browser UA default `margin-inline-start: 40px` (4 terminal columns).
    Dd,
    Table,
    Thead,
    Tbody,
    Tfoot,
    Tr,
    Th,
    Td,
    Caption,
    Colgroup,
    Col,
    Input,
    Form,
    Select,
    HtmlOption,
    /// `<optgroup>` — a labelled group of `<option>`s inside a `<select>`. The
    /// dropdown renders the `label` attribute as a non-selectable group header
    /// row, with the group's options indented beneath it.
    Optgroup,
    /// `<datalist>` — a hidden suggestion source for `<input list="…">`.
    /// The element never renders; its `<option>` children feed the
    /// autocomplete dropdown of matching fields.
    Datalist,
    Button,
    Textarea,
    Label,
    /// `<output>` — the result of a form calculation. Inline element that
    /// renders its text content; the `value` IDL property reads / replaces
    /// that text (like a browser).
    Output,
    /// `<fieldset>` — a labelled group of form controls. Block element with
    /// the browser UA default border; the first `<legend>` child renders as a
    /// plain block inside it.
    Fieldset,
    /// `<legend>` — the caption of a `<fieldset>`. Plain block element.
    Legend,
    /// `<progress>` — a progress indicator. Block element that renders a
    /// terminal bar (`█` filled / `░` empty) from the `value` / `max`
    /// attributes.
    Progress,
    /// `<meter>` — a scalar gauge. Renders like `<progress>`, but the bar is
    /// coloured by the value's position against `low` / `high` / `optimum`.
    Meter,
    /// `<noscript>` — content shown only when scripting is disabled. This
    /// engine always runs its script layer, so the element never renders
    /// (mirroring a browser with JavaScript enabled); the content stays in
    /// the DOM and remains addressable by JS / CSS.
    Noscript,
    /// `<template>` — inert markup for later reuse. The element never renders
    /// (its content is a document fragment by spec); the subtree stays in the
    /// DOM and remains addressable by JS / CSS.
    Template,
    Style,
    Link,
}

impl NodeType {
    /// Every [`NodeType`] variant, in declaration order. The single
    /// enumeration used by [`NodeType::from_tag_name`] and by completeness
    /// tests that prove every element variant is addressable (CSS type
    /// selector, HTML parser, …).
    pub const ALL: &[NodeType] = &[
        NodeType::Document,
        NodeType::DocumentType,
        NodeType::DocumentFragment,
        NodeType::Text,
        NodeType::Comment,
        NodeType::Span,
        NodeType::Div,
        NodeType::P,
        NodeType::Hr,
        NodeType::Pre,
        NodeType::Img,
        NodeType::A,
        NodeType::Em,
        NodeType::Strong,
        NodeType::B,
        NodeType::I,
        NodeType::U,
        NodeType::S,
        NodeType::Del,
        NodeType::Ins,
        NodeType::Mark,
        NodeType::Q,
        NodeType::Abbr,
        NodeType::Cite,
        NodeType::Dfn,
        NodeType::Var,
        NodeType::Code,
        NodeType::Kbd,
        NodeType::Samp,
        NodeType::Data,
        NodeType::Time,
        NodeType::Br,
        NodeType::Html,
        NodeType::Body,
        NodeType::Head,
        NodeType::Title,
        NodeType::Meta,
        NodeType::Base,
        NodeType::H1,
        NodeType::H2,
        NodeType::H3,
        NodeType::H4,
        NodeType::H5,
        NodeType::H6,
        NodeType::HGroup,
        NodeType::Address,
        NodeType::BlockQuote,
        NodeType::Header,
        NodeType::Footer,
        NodeType::Nav,
        NodeType::Main,
        NodeType::Section,
        NodeType::Article,
        NodeType::Aside,
        NodeType::Search,
        NodeType::Figure,
        NodeType::Figcaption,
        NodeType::Details,
        NodeType::Summary,
        NodeType::Dialog,
        NodeType::Ul,
        NodeType::Ol,
        NodeType::Li,
        NodeType::Menu,
        NodeType::Dl,
        NodeType::Dt,
        NodeType::Dd,
        NodeType::Table,
        NodeType::Thead,
        NodeType::Tbody,
        NodeType::Tfoot,
        NodeType::Tr,
        NodeType::Th,
        NodeType::Td,
        NodeType::Caption,
        NodeType::Colgroup,
        NodeType::Col,
        NodeType::Input,
        NodeType::Form,
        NodeType::Select,
        NodeType::HtmlOption,
        NodeType::Optgroup,
        NodeType::Datalist,
        NodeType::Button,
        NodeType::Textarea,
        NodeType::Label,
        NodeType::Output,
        NodeType::Fieldset,
        NodeType::Legend,
        NodeType::Progress,
        NodeType::Meter,
        NodeType::Noscript,
        NodeType::Template,
        NodeType::Style,
        NodeType::Link,
    ];

    /// True for void elements that never have children.
    pub fn is_self_closing(&self) -> bool {
        matches!(
            self,
            NodeType::Img
                | NodeType::Input
                | NodeType::Meta
                | NodeType::Base
                | NodeType::Link
                | NodeType::Hr
                | NodeType::Br
        )
    }

    /// Browser-style UA `display` default for this element.
    ///
    /// Returns the `display` value the element would carry if neither
    /// author CSS nor an inherited value has set `display` on it. The
    /// matches mirror the HTML5 spec / browser defaults:
    ///
    /// - `<img>` → `inline-block` (it's a replaced element)
    /// - `<table>` / `inline-table`-family → their table keyword
    /// - `<li>` → `list-item`
    /// - everything else → `flow` (block-level block container)
    pub fn ua_default_display(&self) -> CssDisplay {
        match self {
            NodeType::Img => CssDisplay::InlineBlock,
            // Inline text-level elements: `<span>` / `<a>` / `<em>` participate
            // in the parent's inline flow. Classifying them block-level would
            // make intrinsic measurement sum each one as a full block row and
            // inflate flex-item / column heights.
            NodeType::Span
            | NodeType::A
            | NodeType::Em
            | NodeType::Strong
            | NodeType::B
            | NodeType::I
            | NodeType::U
            | NodeType::S
            | NodeType::Del
            | NodeType::Ins
            | NodeType::Mark
            | NodeType::Q
            | NodeType::Abbr
            | NodeType::Cite
            | NodeType::Dfn
            | NodeType::Var
            | NodeType::Code
            | NodeType::Kbd
            | NodeType::Samp
            | NodeType::Data
            | NodeType::Time
            | NodeType::Label
            | NodeType::Output
            | NodeType::Br => CssDisplay::Inline,
            NodeType::Table => CssDisplay::Table,
            NodeType::Thead => CssDisplay::TableHeaderGroup,
            NodeType::Tbody => CssDisplay::TableRowGroup,
            NodeType::Tfoot => CssDisplay::TableFooterGroup,
            NodeType::Tr => CssDisplay::TableRow,
            NodeType::Th | NodeType::Td => CssDisplay::TableCell,
            NodeType::Caption => CssDisplay::TableCaption,
            NodeType::Colgroup => CssDisplay::TableColumnGroup,
            NodeType::Col => CssDisplay::TableColumn,
            NodeType::Li => CssDisplay::ListItem,
            _ => CssDisplay::Flow,
        }
    }

    /// Browser user-agent declarations for styled elements.
    ///
    /// Returns the CSS `(property, value)` pairs the UA stylesheet applies when
    /// neither an author rule nor an inherited value has set the property. The
    /// caller injects each pair only when [`CssAttr::is_set`](crate::html::css_parse::CssAttr::is_set) reports it unset.
    ///
    /// `<abbr>`'s underline additionally depends on a `title` attribute and is
    /// handled by the caller; `<q>`'s generated quotes are not CSS declarations
    /// and are handled by the layout code. Despite the name, the table also
    /// carries `<th>`'s UA defaults (bold + centred) for the table-cell path.
    pub fn inline_ua_declarations(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            NodeType::Em | NodeType::I | NodeType::Cite | NodeType::Dfn | NodeType::Var => {
                &[("font-style", "italic")]
            }
            NodeType::Strong | NodeType::B => &[("font-weight", "bold")],
            NodeType::U | NodeType::Ins => &[("text-decoration", "underline")],
            NodeType::S | NodeType::Del => &[("text-decoration", "line-through")],
            NodeType::Mark => &[("color", "black"), ("background-color", "yellow")],
            NodeType::A => &[("color", "blue"), ("text-decoration", "underline")],
            // Browser UA stylesheet: `th { font-weight: bold; text-align: center }`.
            NodeType::Th => &[("font-weight", "bold"), ("text-align", "center")],
            _ => &[],
        }
    }

    /// CSS default `font-size` in `em` units for this element type.
    ///
    /// Browser defaults for headings (CSS 2.1 / user-agent stylesheet):
    /// `h1 = 2em`, `h2 = 1.5em`, `h3 = 1.17em`, `h4 = 1em`,
    /// `h5 = 0.83em`, `h6 = 0.67em`. Every other element inherits the
    /// parent's font-size (returns `None`).
    ///
    /// Values are rounded toward the nearest integer cell when applied
    /// (`1.5em` → 2 cells, `0.83em` → 1 cell); see
    /// [`effective_font_size_cells`](crate::tdom::node::effective_font_size_cells).
    pub fn default_font_size_em(&self) -> Option<f32> {
        match self {
            NodeType::H1 => Some(2.0),
            NodeType::H2 => Some(1.5),
            NodeType::H3 => Some(1.17),
            NodeType::H4 => Some(1.0),
            NodeType::H5 => Some(0.83),
            NodeType::H6 => Some(0.67),
            _ => None,
        }
    }

    /// Browser-style UA `white-space` default for this element type.
    ///
    /// Mirrors the HTML5 user-agent stylesheet: `<pre>` ships with
    /// `white-space: pre` so leading whitespace, tabs, and newlines are
    /// preserved verbatim. Every other element returns `None` — the
    /// layout falls back to the regular `WhiteSpace::Normal` default.
    pub fn default_white_space(&self) -> Option<WhiteSpace> {
        match self {
            NodeType::Pre => Some(WhiteSpace::Pre),
            _ => None,
        }
    }

    /// Lowercase HTML tag name for element variants, `None` for the
    /// document-structure nodes (`Document` / `DocumentType` /
    /// `DocumentFragment` / `Text` / `Comment`).
    pub fn tag_name(&self) -> Option<&'static str> {
        Some(match self {
            NodeType::Span => "span",
            NodeType::Div => "div",
            NodeType::P => "p",
            NodeType::Hr => "hr",
            NodeType::Pre => "pre",
            NodeType::Img => "img",
            NodeType::A => "a",
            NodeType::Em => "em",
            NodeType::Strong => "strong",
            NodeType::B => "b",
            NodeType::I => "i",
            NodeType::U => "u",
            NodeType::S => "s",
            NodeType::Del => "del",
            NodeType::Ins => "ins",
            NodeType::Mark => "mark",
            NodeType::Q => "q",
            NodeType::Abbr => "abbr",
            NodeType::Cite => "cite",
            NodeType::Dfn => "dfn",
            NodeType::Var => "var",
            NodeType::Code => "code",
            NodeType::Kbd => "kbd",
            NodeType::Samp => "samp",
            NodeType::Data => "data",
            NodeType::Time => "time",
            NodeType::Br => "br",
            NodeType::Html => "html",
            NodeType::Body => "body",
            NodeType::Head => "head",
            NodeType::Title => "title",
            NodeType::Meta => "meta",
            NodeType::Base => "base",
            NodeType::H1 => "h1",
            NodeType::H2 => "h2",
            NodeType::H3 => "h3",
            NodeType::H4 => "h4",
            NodeType::H5 => "h5",
            NodeType::H6 => "h6",
            NodeType::HGroup => "hgroup",
            NodeType::Address => "address",
            NodeType::BlockQuote => "blockquote",
            NodeType::Header => "header",
            NodeType::Footer => "footer",
            NodeType::Nav => "nav",
            NodeType::Main => "main",
            NodeType::Section => "section",
            NodeType::Article => "article",
            NodeType::Aside => "aside",
            NodeType::Search => "search",
            NodeType::Figure => "figure",
            NodeType::Figcaption => "figcaption",
            NodeType::Details => "details",
            NodeType::Summary => "summary",
            NodeType::Dialog => "dialog",
            NodeType::Ul => "ul",
            NodeType::Ol => "ol",
            NodeType::Li => "li",
            NodeType::Menu => "menu",
            NodeType::Dl => "dl",
            NodeType::Dt => "dt",
            NodeType::Dd => "dd",
            NodeType::Table => "table",
            NodeType::Thead => "thead",
            NodeType::Tbody => "tbody",
            NodeType::Tfoot => "tfoot",
            NodeType::Tr => "tr",
            NodeType::Th => "th",
            NodeType::Td => "td",
            NodeType::Caption => "caption",
            NodeType::Colgroup => "colgroup",
            NodeType::Col => "col",
            NodeType::Input => "input",
            NodeType::Form => "form",
            NodeType::Select => "select",
            NodeType::HtmlOption => "option",
            NodeType::Optgroup => "optgroup",
            NodeType::Datalist => "datalist",
            NodeType::Button => "button",
            NodeType::Textarea => "textarea",
            NodeType::Label => "label",
            NodeType::Output => "output",
            NodeType::Fieldset => "fieldset",
            NodeType::Legend => "legend",
            NodeType::Progress => "progress",
            NodeType::Meter => "meter",
            NodeType::Noscript => "noscript",
            NodeType::Template => "template",
            NodeType::Style => "style",
            NodeType::Link => "link",
            _ => return None,
        })
    }

    /// Reverse of [`NodeType::tag_name`]: the element variant whose lowercase
    /// tag name is `tag`, or `None` when `tag` does not name a supported
    /// element (document-structure nodes have no tag name and never match).
    ///
    /// Derived from [`NodeType::ALL`] + [`NodeType::tag_name`] so there is
    /// exactly one tag table to keep in sync. The CSS selector parser
    /// (`crate::html::css_parse::selector`) and the HTML parser
    /// ([`crate::html::html_parse::tag_name_to_node_type`]) both delegate
    /// here.
    pub fn from_tag_name(tag: &str) -> Option<NodeType> {
        let t = tag.to_ascii_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|nt| nt.tag_name() == Some(t.as_str()))
    }
}

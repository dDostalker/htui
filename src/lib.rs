pub mod html;
pub mod layout;
pub mod tdom;
pub mod tui;

use std::io::{Write, stdout};

use anyhow::Result;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};

pub use html::html_parse::Parser;
pub use layout::layout::TermMap;

use crate::{
    html::css_parse,
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::{TermLayout, trigger_refresh},
};

/// 解析 HTML + CSS → (root_id, arena)
fn parse(
    html: &mut html::html_parse::Parser,
    css: &mut css_parse::Parser,
    external_css: &str,
) -> Result<(NodeId, NodeArena)> {
    // HTML 解析 → 含嵌入 CSS
    let (root_id, mut arena, embedded_css) = html.parse()?;

    // 拼接嵌入 CSS + 外部 CSS
    let combined = if external_css.is_empty() {
        embedded_css
    } else {
        format!("{}\n{}", embedded_css, external_css)
    };

    // 解析并应用 CSS
    if !combined.trim().is_empty() {
        *css = css_parse::Parser::new(&combined);
        let rules = css.parse()?;
        css_parse::css_into_node(&mut arena, root_id, &rules)?;
    }

    Ok((root_id, arena))
}

/// 渲染入口
pub fn render(html: &str, css: &str) {
    enable_raw_mode().unwrap();
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, Hide).unwrap();

    'remder: loop {
        execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)).unwrap();

        let mut html_parser = html::html_parse::Parser::new(html);
        let mut css_parser = css_parse::Parser::new("");
        let (root_id, arena) = parse(&mut html_parser, &mut css_parser, css).unwrap();

        let term_map = TermMap::get_ter_map().unwrap();
        let mut term_layout = TermLayout::new(&term_map).unwrap();
        term_layout.draw(&arena, root_id, &term_map).unwrap();

        term_layout.render();
        stdout().flush().unwrap();

        loop {
            match trigger_refresh() {
                Some(false) => break 'remder,
                None => {}
                Some(true) => break,
            }
        }
    }

    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, Show).unwrap();
    disable_raw_mode().unwrap();
}

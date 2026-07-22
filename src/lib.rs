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
    tui::graph_api::{RefreshAction, TermLayout, trigger_refresh},
};

/// Parse HTML + CSS into (root_id, arena)
fn parse(
    html: &mut html::html_parse::Parser,
    css: &mut css_parse::Parser,
    external_css: &str,
) -> Result<(NodeId, NodeArena)> {
    let (root_id, mut arena, embedded_css) = html.parse()?;

    let combined = if external_css.is_empty() {
        embedded_css
    } else {
        format!("{}\n{}", embedded_css, external_css)
    };

    if !combined.trim().is_empty() {
        *css = css_parse::Parser::new(&combined);
        let rules = css.parse()?;
        css_parse::css_into_node(&mut arena, root_id, &rules)?;
    }

    Ok((root_id, arena))
}

/// Render entry point
pub fn render(html: &str, css: &str) {
    enable_raw_mode().unwrap();
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, Hide).unwrap();

    let mut zoom: f32 = 1.0;
    let mut saved_scroll_y: u32 = 0;

    'remder: loop {
        execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)).unwrap();

        let mut html_parser = html::html_parse::Parser::new(html);
        let mut css_parser = css_parse::Parser::new("");
        let (root_id, arena) = parse(&mut html_parser, &mut css_parser, css).unwrap();

        let term_map = TermMap::get_ter_map().unwrap();
        let mut term_layout = TermLayout::new(&term_map, zoom).unwrap();
        term_layout.draw(&arena, root_id, &term_map).unwrap();

        // 恢复缩放前的滚动位置
        term_layout.scroll_y = saved_scroll_y;
        term_layout.clamp_scroll();

        term_layout.render();
        stdout().flush().unwrap();

        loop {
            match trigger_refresh(&mut term_layout, &mut zoom) {
                RefreshAction::Quit => break 'remder,
                RefreshAction::Redraw => {
                    saved_scroll_y = term_layout.scroll_y;
                    break;
                }
                RefreshAction::Rerender => {
                    execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)).unwrap();
                    term_layout.render();
                    stdout().flush().unwrap();
                }
                RefreshAction::Continue => {}
            }
        }
    }

    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, Show).unwrap();
    disable_raw_mode().unwrap();
}

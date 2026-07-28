pub mod html;
pub mod layout;
pub mod logging;
pub mod tdom;
pub mod tui;

use std::io::{Write, stdout};
use std::sync::Arc;
use std::sync::mpsc;

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
    html::css_parse::{self, CssRule},
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::{RenderCmd, TermLayout, trigger_refresh},
};

fn parse_and_apply(html: &str, css_rules: &CssRule) -> (NodeId, NodeArena) {
    let mut html_parser = html::html_parse::Parser::new(html);
    let (root_id, mut arena, _embedded) = html_parser.parse();
    css_parse::css_into_node(&mut arena, root_id, css_rules);
    (root_id, arena)
}

fn build_css_rules(html: &str, css: &str) -> CssRule {
    let mut html_parser = html::html_parse::Parser::new(html);
    let (_root_id, _arena, embedded_css) = html_parser.parse();

    let combined = if css.is_empty() {
        embedded_css
    } else {
        format!("{}\n{}", embedded_css, css)
    };

    if combined.trim().is_empty() {
        return CssRule::new();
    }

    let mut css_parser = css_parse::Parser::new(&combined);
    css_parser.parse()
}

// ============================================================
// Render thread
// ============================================================

fn render_thread(rx: mpsc::Receiver<RenderCmd>, html: Arc<str>, css_rules: Arc<CssRule>) {
    let mut term_layout: Option<TermLayout> = None;

    loop {
        let mut cmd = match rx.recv() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Render channel disconnected: {}", e);
                break;
            }
        };
        while let Ok(next) = rx.try_recv() {
            cmd = next;
            if matches!(cmd, RenderCmd::Quit) {
                break;
            }
        }

        match cmd {
            RenderCmd::Quit => break,

            RenderCmd::Rerender { scroll_y } => {
                if let Some(ref mut tl) = term_layout {
                    tl.scroll_y = scroll_y;
                    tl.clamp_scroll();
                    if let Err(e) = execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)) {
                        log::error!("Failed to clear screen: {}", e);
                    }
                    tl.render();
                    if let Err(e) = stdout().flush() {
                        log::error!("Failed to flush stdout: {}", e);
                    }
                }
            }

            RenderCmd::Redraw { scroll_y, zoom } => {
                let (root_id, arena) = parse_and_apply(&html, &css_rules);

                let term_map = match TermMap::get_ter_map() {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("Failed to get terminal size: {}", e);
                        continue;
                    }
                };
                let mut tl = TermLayout::new(&term_map, zoom);
                if let Err(e) = tl.draw(&arena, root_id, &term_map) {
                    log::error!("Layout failed: {}", e);
                    continue;
                }

                tl.scroll_y = scroll_y;
                tl.clamp_scroll();

                if let Err(e) = execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)) {
                    log::error!("Failed to clear screen: {}", e);
                }
                tl.render();
                if let Err(e) = stdout().flush() {
                    log::error!("Failed to flush stdout: {}", e);
                }

                term_layout = Some(tl);
            }
        }
    }
}

// ============================================================
// Entry point
// ============================================================

pub fn render(html: &str, css: &str) {
    if let Err(e) = enable_raw_mode() {
        log::error!("Failed to enable raw mode: {}", e);
        return;
    }
    if let Err(e) = execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, Hide) {
        log::error!("Failed to enter alternate screen: {}", e);
        if let Err(e2) = disable_raw_mode() {
            log::error!("Failed to disable raw mode during cleanup: {}", e2);
        }
        return;
    }

    let css_rules = Arc::new(build_css_rules(html, css));

    let html: Arc<str> = Arc::from(html);
    let (tx, rx) = mpsc::channel::<RenderCmd>();

    let render_handle = std::thread::spawn({
        let html_clone = Arc::clone(&html);
        let css_clone = Arc::clone(&css_rules);
        move || {
            render_thread(rx, html_clone, css_clone);
        }
    });

    let mut scroll_y: u32 = 0;
    let mut zoom: f32 = 1.0;
    let mut rows: u16 = match TermMap::get_ter_map() {
        Ok(m) => m.rows,
        Err(e) => {
            log::error!("Failed to get terminal size: {}", e);
            24
        }
    };

    if let Err(e) = tx.send(RenderCmd::Redraw { scroll_y, zoom }) {
        log::error!("Failed to send initial Redraw: {}", e);
        if let Err(e2) = tx.send(RenderCmd::Quit) {
            log::error!("Failed to send Quit during cleanup: {}", e2);
        }
    }

    loop {
        match trigger_refresh(&mut scroll_y, &mut zoom, rows) {
            Some(RenderCmd::Quit) => {
                if let Err(e) = tx.send(RenderCmd::Quit) {
                    log::error!("Failed to send Quit: {}", e);
                }
                break;
            }
            Some(cmd) => {
                if let RenderCmd::Redraw { .. } = &cmd {
                    if let Ok(map) = TermMap::get_ter_map() {
                        rows = map.rows;
                    }
                }
                if let Err(e) = tx.send(cmd) {
                    log::error!("Failed to send render command: {}", e);
                    break;
                }
            }
            None => {}
        }
    }

    if let Err(e) = render_handle.join() {
        log::error!("Render thread panicked: {:?}", e);
    }

    if let Err(e) = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, Show) {
        log::error!("Failed to leave alternate screen: {}", e);
    }
    if let Err(e) = disable_raw_mode() {
        log::error!("Failed to disable raw mode: {}", e);
    }
}

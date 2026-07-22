pub mod html;
pub mod layout;
pub mod tdom;
pub mod tui;

use std::io::{Write, stdout};
use std::sync::mpsc;
use std::sync::Arc;

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
    html::css_parse::{self, CssRule},
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::{RenderCmd, TermLayout, trigger_refresh},
};

fn parse_and_apply(html: &str, css_rules: &CssRule) -> Result<(NodeId, NodeArena)> {
    let mut html_parser = html::html_parse::Parser::new(html);
    let (root_id, mut arena, _embedded) = html_parser.parse()?;
    css_parse::css_into_node(&mut arena, root_id, css_rules)?;
    Ok((root_id, arena))
}

fn build_css_rules(html: &str, css: &str) -> Result<CssRule> {
    let mut html_parser = html::html_parse::Parser::new(html);
    let (_root_id, _arena, embedded_css) = html_parser.parse()?;

    let combined = if css.is_empty() {
        embedded_css
    } else {
        format!("{}\n{}", embedded_css, css)
    };

    if combined.trim().is_empty() {
        return Ok(CssRule::new());
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
        let mut cmd = rx.recv().unwrap();
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
                    execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)).unwrap();
                    tl.render();
                    stdout().flush().unwrap();
                }
            }

            RenderCmd::Redraw { scroll_y, zoom } => {
                let (root_id, arena) = match parse_and_apply(&html, &css_rules) {
                    Ok(v) => v,
                    Err(e) => {
                        log::error!("Parse failed: {}", e);
                        continue;
                    }
                };

                let term_map = TermMap::get_ter_map().unwrap();
                let mut tl = TermLayout::new(&term_map, zoom).unwrap();
                if let Err(e) = tl.draw(&arena, root_id, &term_map) {
                    log::error!("Layout failed: {}", e);
                    continue;
                }

                tl.scroll_y = scroll_y;
                tl.clamp_scroll();

                execute!(stdout(), Clear(ClearType::All), MoveTo(0, 0)).unwrap();
                tl.render();
                stdout().flush().unwrap();

                term_layout = Some(tl);
            }
        }
    }
}

// ============================================================
// Entry point
// ============================================================

pub fn render(html: &str, css: &str) {
    enable_raw_mode().unwrap();
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, Hide).unwrap();

    let css_rules = match build_css_rules(html, css) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            log::error!("CSS parse failed: {}", e);
            Arc::new(CssRule::new())
        }
    };

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
    let mut rows: u16 = TermMap::get_ter_map().unwrap().rows;

    tx.send(RenderCmd::Redraw { scroll_y, zoom }).unwrap();

    loop {
        match trigger_refresh(&mut scroll_y, &mut zoom, rows) {
            Some(RenderCmd::Quit) => {
                tx.send(RenderCmd::Quit).unwrap();
                break;
            }
            Some(cmd) => {
                if let RenderCmd::Redraw { .. } = &cmd {
                    if let Ok(map) = TermMap::get_ter_map() {
                        rows = map.rows;
                    }
                }
                tx.send(cmd).unwrap();
            }
            None => {}
        }
    }

    render_handle.join().unwrap();

    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, Show).unwrap();
    disable_raw_mode().unwrap();
}

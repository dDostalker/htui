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
    html::css_parse,
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::{RenderCmd, TermLayout, trigger_refresh},
};

/// Parse HTML + CSS into (root_id, arena)
fn parse(
    html: &str,
    css: &str,
) -> Result<(NodeId, NodeArena)> {
    let mut html_parser = html::html_parse::Parser::new(html);
    let (root_id, mut arena, embedded_css) = html_parser.parse()?;

    let combined = if css.is_empty() {
        embedded_css
    } else {
        format!("{}\n{}", embedded_css, css)
    };

    if !combined.trim().is_empty() {
        let mut css_parser = css_parse::Parser::new(&combined);
        let rules = css_parser.parse()?;
        css_parse::css_into_node(&mut arena, root_id, &rules)?;
    }

    Ok((root_id, arena))
}

// ============================================================
// 渲染线程
// ============================================================

fn render_thread(rx: mpsc::Receiver<RenderCmd>, html: Arc<str>, css: Arc<str>) {
    let mut term_layout: Option<TermLayout> = None;

    loop {
        // 阻塞等待第一个命令
        let mut cmd = rx.recv().unwrap();

        // 排空队列，只保留最后一条（合并中间帧）
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
                let (root_id, arena) = match parse(&html, &css) {
                    Ok(v) => v,
                    Err(e) => {
                        log::error!("解析失败: {}", e);
                        continue;
                    }
                };

                let term_map = TermMap::get_ter_map().unwrap();
                let mut tl = TermLayout::new(&term_map, zoom).unwrap();
                if let Err(e) = tl.draw(&arena, root_id, &term_map) {
                    log::error!("布局失败: {}", e);
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
// 入口
// ============================================================

/// Render entry point
pub fn render(html: &str, css: &str) {
    enable_raw_mode().unwrap();
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, Hide).unwrap();

    let html: Arc<str> = Arc::from(html);
    let css: Arc<str> = Arc::from(css);

    let (tx, rx) = mpsc::channel::<RenderCmd>();

    // 启动渲染线程
    let render_handle = std::thread::spawn({
        let html_clone = Arc::clone(&html);
        let css_clone = Arc::clone(&css);
        move || {
            render_thread(rx, html_clone, css_clone);
        }
    });

    // 主线程：事件读取 + 发送命令
    let mut scroll_y: u32 = 0;
    let mut zoom: f32 = 1.0;
    let mut rows: u16 = TermMap::get_ter_map().unwrap().rows;

    // 初始渲染
    tx.send(RenderCmd::Redraw { scroll_y, zoom }).unwrap();

    loop {
        match trigger_refresh(&mut scroll_y, &mut zoom, rows) {
            Some(RenderCmd::Quit) => {
                tx.send(RenderCmd::Quit).unwrap();
                break;
            }
            Some(cmd) => {
                // 更新行数（来自 Resize 事件的新值）
                if let RenderCmd::Redraw { .. } = &cmd {
                    if let Ok(map) = TermMap::get_ter_map() {
                        rows = map.rows;
                    }
                }
                tx.send(cmd).unwrap();
            }
            None => {} // 无操作，继续等待
        }
    }

    render_handle.join().unwrap();

    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, Show).unwrap();
    disable_raw_mode().unwrap();
}

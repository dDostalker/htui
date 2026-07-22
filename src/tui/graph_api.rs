use std::{
    cell::RefCell,
    fmt::{self, Display},
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use anyhow::{Result, anyhow};
use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    style::{Color, Stylize},
};
use image::{
    GenericImageView,
    imageops::FilterType::{self},
};
use log::{debug, error, warn};
use std::io::stdout;

/// Maximum virtual canvas multiplier (relative to terminal viewport)
const MAX_SCROLL_MULTIPLIER: usize = 20;

/// Mouse wheel scroll lines per tick
const SCROLL_LINES_PER_TICK: u32 = 3;

// ============================================================
// Render command
// ============================================================

#[derive(Debug, Clone)]
pub enum RenderCmd {
    Rerender { scroll_y: u32 },
    Redraw { scroll_y: u32, zoom: f32 },
    Quit,
}

// ============================================================
// char_width
// ============================================================

pub fn char_width(c: char) -> usize {
    if c.is_ascii() {
        return 1;
    }
    let cp = c as u32;
    let is_wide = matches!(cp,
        0x4E00..=0x9FFF
        | 0x3400..=0x4DBF
        | 0xF900..=0xFAFF
        | 0x2F800..=0x2FA1F
        | 0xFF01..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0xAC00..=0xD7AF
        | 0x3000..=0x303F
        | 0x2E80..=0x2EFF
        | 0x3200..=0x33FF
        | 0xFFE8..=0xFFEE
    );
    if is_wide { 2 } else { 1 }
}

// ============================================================
// Block
// ============================================================

#[derive(Debug, Clone)]
pub struct Block {
    pub word: String,
    pub fcolor: Option<Color>,
    pub bcolor: Option<Color>,
    render: Option<RefCell<String>>,
}

impl Deref for Block {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.word
    }
}

impl DerefMut for Block {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.word
    }
}

impl Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_render() {
            write!(f, "{}", self.render_str())?;
        } else {
            let styled = self.word.clone().stylize();
            let styled = match (self.fcolor, self.bcolor) {
                (Some(fg), Some(bg)) => styled.with(fg).on(bg),
                (Some(fg), None) => styled.with(fg),
                (None, Some(bg)) => styled.on(bg),
                (None, None) => styled,
            };
            write!(f, "{styled}")?;
        }
        Ok(())
    }
}

impl Block {
    fn is_render(&self) -> bool {
        self.render.is_some()
    }
    pub fn new(word: String, fcolor: Option<Color>, bcolor: Option<Color>) -> Self {
        Self {
            word,
            fcolor,
            bcolor,
            render: None,
        }
    }
    pub fn new_render(
        word: String,
        fcolor: Option<Color>,
        bcolor: Option<Color>,
        render: Option<String>,
    ) -> Self {
        match render {
            Some(render) => Self {
                word,
                fcolor,
                bcolor,
                render: Some(RefCell::new(render)),
            },
            None => Self {
                word,
                fcolor,
                bcolor,
                render: None,
            },
        }
    }
    fn render_str(&self) -> String {
        match self.render.clone() {
            Some(render) => render.borrow().as_str().to_string(),
            None => {
                debug!("render not styled");
                self.word.clone()
            }
        }
    }
}

// ============================================================
// TermLayout
// ============================================================

#[derive(Debug)]
pub struct TermLayout {
    pub layout: Vec<Block>,
    pub point: u32,
    pub len: u32,
    pub columns: u16,
    pub rows: u16,
    pub scroll_y: u32,
    pub rendered_rows: u32,
    pub zoom: f32,
}

impl TermLayout {
    pub fn max_cells(&self) -> usize {
        self.columns as usize * self.rows as usize * MAX_SCROLL_MULTIPLIER
    }

    pub fn is_full(&self) -> bool {
        self.point as usize >= self.max_cells()
    }

    pub fn effective_columns(&self) -> u16 {
        (self.columns as f32 / self.zoom).round() as u16
    }

    pub fn effective_rows(&self) -> u16 {
        (self.rows as f32 / self.zoom).round() as u16
    }

    pub fn max_scroll(&self) -> u32 {
        let total_lines = self.rendered_rows.max(self.point / self.columns as u32);
        total_lines.saturating_sub(self.rows as u32)
    }

    pub fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        if self.scroll_y > max {
            self.scroll_y = max;
        }
    }
}

// ============================================================
// Display (debug)
// ============================================================

impl Display for TermLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cols = self.columns as usize;
        let rows = self.rows as usize;
        let sy = self.scroll_y as usize;
        for r in 0..rows {
            let start = (sy + r) * cols;
            let end = (start + cols).min(self.layout.len());
            for cell in self.layout[start..end].iter() {
                if cell.word != " " {
                    write!(f, "{}", cell)?;
                }
            }
        }
        write!(f, "")
    }
}

// ============================================================
// Render (viewport)
// ============================================================

impl TermLayout {
    pub fn render(&self) {
        let cols = self.columns as usize;
        let rows = self.rows as usize;
        let scroll_start = self.scroll_y as usize * cols;
        let scroll_end = (scroll_start + rows * cols).min(self.layout.len());

        let mut i = scroll_start;

        while i < scroll_end {
            // 跳过宽字符的填充格（空字符串标记）
            if self.layout[i].word.is_empty() {
                i += 1;
                continue;
            }

            let cell = &self.layout[i];

            let batch_start = i;
            let fg = cell.fcolor;
            let bg = cell.bcolor;
            let is_empty = cell.word == " " && cell.bcolor.is_none();
            let mut batch = String::new();

            while i < scroll_end {
                let c = &self.layout[i];
                // 跳过宽字符的填充格
                if c.word.is_empty() {
                    i += 1;
                    continue;
                }
                if i > batch_start && i % cols == 0 {
                    break;
                }
                let c_empty = c.word == " " && c.bcolor.is_none();
                if c_empty != is_empty {
                    break;
                }
                if !c_empty && (c.fcolor != fg || c.bcolor != bg) {
                    break;
                }
                batch.push_str(&c.word);
                i += 1;
            }

            let x = (batch_start % cols) as u16;
            let y = (batch_start / cols) as u16 - self.scroll_y as u16;

            if y < self.rows {
                execute!(stdout(), MoveTo(x, y)).ok();
                if is_empty {
                    print!("{batch}");
                } else {
                    let styled = batch.stylize();
                    let styled = match (fg, bg) {
                        (Some(f), Some(b)) => styled.with(f).on(b),
                        (Some(f), None) => styled.with(f),
                        (None, Some(b)) => styled.on(b),
                        (None, None) => styled,
                    };
                    print!("{styled}");
                }
            }
        }
    }
}

// ============================================================
// Paint methods
// ============================================================

impl TermLayout {
    pub fn new_line(&mut self) {
        if self.is_full() {
            return;
        }
        let columns = self.columns as u32;
        let remaining = columns - (self.point % columns);
        let new_point = (self.point + remaining).min(self.max_cells() as u32);
        self.point = new_point;
    }

    pub fn paint_string(&mut self, s: &str, fcolor: Option<Color>, bcolor: Option<Color>) -> u32 {
        if self.is_full() {
            return 0;
        }

        let max = self.max_cells();
        let mut cell_count = 0u32;

        for c in s.chars() {
            let idx = self.point as usize;
            if idx >= max {
                break;
            }
            let width = char_width(c);
            self.layout[idx] = Block::new(c.to_string(), fcolor, bcolor);
            if width > 1 {
                let next_idx = idx + 1;
                if next_idx < max {
                    // 填充格用空字符串标记，渲染时跳过，避免终端光标偏移
                    self.layout[next_idx] = Block::new(String::new(), fcolor, bcolor);
                }
            }
            self.point += width as u32;
            cell_count += width as u32;
        }

        self.len += cell_count;

        let current_row = self.point / self.columns as u32;
        if current_row > self.rendered_rows {
            self.rendered_rows = current_row;
        }

        cell_count
    }

    pub fn paint_img(&mut self, path: String, size: (u32, u32)) -> u32 {
        if self.is_full() {
            return 0;
        }

        debug!("Loading image: path='{}' size={:?}", path, size);

        let mut img_data = Img::new(PathBuf::from(&path), size);
        if let Err(e) = img_data._create_img() {
            error!("Image load failed: {}: {}", path, e);
            return 0;
        }
        let picture = match img_data.picture.as_ref() {
            Some(p) => p,
            None => {
                warn!("Image data empty: {}", path);
                return 0;
            }
        };

        let img_cols = img_data.cloumns as usize;
        let img_rows = img_data.rows as usize;
        let canvas_cols = self.columns as usize;
        let max = self.max_cells();

        let start_row = self.point as usize / canvas_cols;

        for row in 0..img_rows {
            for col in 0..img_cols {
                let canvas_idx = (start_row + row) * canvas_cols + col;
                if canvas_idx >= max {
                    warn!("Canvas overflow, row={} col={}", row, col);
                    break;
                }
                let pic_idx = row * img_cols + col;
                if let Some(cell) = picture.get(pic_idx) {
                    self.layout[canvas_idx] = Block::new(cell.clone(), None, None);
                }
            }
        }

        let total_chars = (img_rows * img_cols) as u32;
        self.point = ((start_row + img_rows) as u32 * canvas_cols as u32).min(max as u32);
        self.len += total_chars;

        self.rendered_rows = self.point / self.columns as u32;

        debug!("Image loaded, path={}, chars={}", path, total_chars);
        total_chars
    }
}

// ============================================================
// trigger_refresh
// ============================================================

const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 5.0;

pub fn trigger_refresh(scroll_y: &mut u32, zoom: &mut f32, rows: u16) -> Option<RenderCmd> {
    match event::read() {
        Ok(Event::Key(key)) => {
            if key.kind == KeyEventKind::Release {
                return None;
            }

            if key.modifiers.contains(event::KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('=') | KeyCode::Char('+') => {
                        *zoom = (*zoom + ZOOM_STEP).min(ZOOM_MAX);
                        return Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        });
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        *zoom = (*zoom - ZOOM_STEP).max(ZOOM_MIN);
                        return Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        });
                    }
                    KeyCode::Char('0') => {
                        *zoom = 1.0;
                        return Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        });
                    }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Some(RenderCmd::Quit),

                KeyCode::Up | KeyCode::Char('k') => {
                    *scroll_y = scroll_y.saturating_sub(1);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *scroll_y = scroll_y.saturating_add(1);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::PageUp => {
                    let step = (rows as u32).saturating_sub(1).max(1);
                    *scroll_y = scroll_y.saturating_sub(step);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::PageDown => {
                    let step = (rows as u32).saturating_sub(1).max(1);
                    *scroll_y = scroll_y.saturating_add(step);
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::Home => {
                    *scroll_y = 0;
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                KeyCode::End => {
                    *scroll_y = u32::MAX;
                    return Some(RenderCmd::Rerender {
                        scroll_y: *scroll_y,
                    });
                }
                _ => {}
            }
            None
        }

        Ok(Event::Mouse(mouse)) => {
            let ctrl = mouse.modifiers.contains(event::KeyModifiers::CONTROL);
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    if ctrl {
                        *zoom = (*zoom - ZOOM_STEP).max(ZOOM_MIN);
                        Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        })
                    } else {
                        *scroll_y = scroll_y.saturating_sub(SCROLL_LINES_PER_TICK);
                        Some(RenderCmd::Rerender {
                            scroll_y: *scroll_y,
                        })
                    }
                }
                MouseEventKind::ScrollDown => {
                    if ctrl {
                        *zoom = (*zoom + ZOOM_STEP).min(ZOOM_MAX);
                        Some(RenderCmd::Redraw {
                            scroll_y: *scroll_y,
                            zoom: *zoom,
                        })
                    } else {
                        *scroll_y = scroll_y.saturating_add(SCROLL_LINES_PER_TICK);
                        Some(RenderCmd::Rerender {
                            scroll_y: *scroll_y,
                        })
                    }
                }
                _ => None,
            }
        }

        Ok(Event::Resize(_, _new_rows)) => Some(RenderCmd::Redraw {
            scroll_y: *scroll_y,
            zoom: *zoom,
        }),

        _ => None,
    }
}

// ============================================================
// Img
// ============================================================

#[derive(Debug)]
pub struct Img {
    pub path: PathBuf,
    pub term_size: (u32, u32),
    pub picture: Option<Vec<String>>,
    pub cloumns: u32,
    pub rows: u32,
}

impl Img {
    pub fn new(path: PathBuf, term_size: (u32, u32)) -> Self {
        Self {
            path,
            term_size,
            picture: None,
            cloumns: 0,
            rows: 0,
        }
    }

    pub fn _create_img(&mut self) -> Result<()> {
        if self.picture.is_some() {
            return Ok(());
        }

        let img = image::open(&self.path)?;
        let img = img.resize(self.term_size.0, self.term_size.1, FilterType::Nearest);
        let (width, height) = img.dimensions();

        self.cloumns = width.div_ceil(2);
        self.rows = height.div_ceil(4);
        self.picture = Some(vec![]);
        for y in (0..height).step_by(4) {
            for x in (0..width).step_by(2) {
                let mut bits = 0;
                let mut r_sum = 0u32;
                let mut g_sum = 0u32;
                let mut b_sum = 0u32;
                let mut count = 0;

                for dy in 0..4 {
                    for dx in 0..2 {
                        let px = x + dx;
                        let py = y + dy;

                        if px < width && py < height {
                            let pixel = img.get_pixel(px, py);
                            r_sum += pixel[0] as u32;
                            g_sum += pixel[1] as u32;
                            b_sum += pixel[2] as u32;
                            count += 1;

                            let brightness =
                                (pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16) / 3;
                            if brightness < 128 {
                                let bit_index = match (dx, dy) {
                                    (0, 0) => 0,
                                    (0, 1) => 1,
                                    (0, 2) => 2,
                                    (1, 0) => 3,
                                    (1, 1) => 4,
                                    (1, 2) => 5,
                                    (0, 3) => 6,
                                    (1, 3) => 7,
                                    _ => 0,
                                };
                                bits |= 1 << bit_index;
                            }
                        }
                    }
                }

                r_sum = r_sum.checked_div(count).ok_or(anyhow!("r_sum is 0"))?;
                g_sum = g_sum.checked_div(count).ok_or(anyhow!("g_sum is 0"))?;
                b_sum = b_sum.checked_div(count).ok_or(anyhow!("b_sum is 0"))?;
                let color = Color::Rgb {
                    r: r_sum as u8,
                    g: g_sum as u8,
                    b: b_sum as u8,
                };
                if let Some(picture) = self.picture.as_mut() {
                    let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                    picture.push(format!("{}", ch.to_string().with(color)));
                }
            }
        }
        Ok(())
    }
}

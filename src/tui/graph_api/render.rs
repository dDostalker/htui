use std::{fmt::Display, io::stdout};

use crossterm::cursor::MoveTo;

use super::{
    canvas::TermLayout,
    cell::{Block, styled_string},
};

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
impl TermLayout {
    pub fn render(&self) {
        let bytes = self.render_to_bytes();
        use std::io::Write;
        let mut out = stdout();
        if let Err(e) = out.write_all(&bytes) {
            log::error!("Failed to write rendered frame to stdout: {}", e);
        }
        if let Err(e) = out.flush() {
            log::error!("Failed to flush stdout: {}", e);
        }
    }

    /// Render the visible viewport to an in-memory byte buffer.
    ///
    /// Same output as `render()` but without any terminal I/O — useful for
    /// unit tests that want to inspect the actual ANSI SGR sequence. The
    /// cursor positioning commands are also written (as raw ANSI), so this
    /// is a faithful reproduction of what `render()` writes to stdout.
    pub fn render_to_bytes(&self) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
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
            let cell_style = cell.style;
            // A cell is rendered as a "bare" space only when it has no
            // background AND no style flags. Anything styled (even a plain
            // space) must still receive the SGR codes — otherwise spaces
            // between underlined/italic letters would drop the underline.
            let is_empty = cell.word == " " && cell.bcolor.is_none() && cell.style.is_empty();
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
                let c_empty = c.word == " " && c.bcolor.is_none() && c.style.is_empty();
                if c_empty != is_empty {
                    break;
                }
                if !c_empty && (c.fcolor != fg || c.bcolor != bg || c.style != cell_style) {
                    break;
                }
                batch.push_str(&c.word);
                i += 1;
            }

            let x = (batch_start % cols) as u16;
            let y = (batch_start / cols) as u16 - self.scroll_y as u16;

            if y < self.rows {
                if let Err(e) = crossterm::queue!(out, MoveTo(x, y)) {
                    log::error!("Failed to queue cursor move to ({}, {}): {}", x, y, e);
                }
                if is_empty {
                    out.extend_from_slice(batch.as_bytes());
                } else {
                    out.extend_from_slice(styled_string(&batch, cell_style, fg, bg).as_bytes());
                }
            }
        }
        out
    }
}

/// Returns true when a cell should be rendered as a "bare" space without
/// applying any SGR escape codes. Equivalent to the inline check in
/// `TermLayout::render_to_bytes`; exposed for tests.
pub fn is_rendered_as_bare_space(cell: &Block) -> bool {
    cell.word == " " && cell.bcolor.is_none() && cell.style.is_empty()
}

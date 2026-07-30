use std::path::PathBuf;

use anyhow::{Result, anyhow};
use crossterm::style::{Color, Stylize};
use image::{GenericImageView, imageops::FilterType};

use super::{
    canvas::TermLayout,
    cell::{Block, FontStyle},
};

// ============================================================
// Image pixel -> terminal cell sampling ratios.
//
// These constants define how source-image pixels map onto terminal
// cells. They are intentionally independent of
// `crate::html::css_parse::values::PX_PER_CELL`, which governs CSS px
// in the box model. The two answer different questions:
//   - PX_PER_CELL: how many CSS px is one terminal cell? Answer: 1.
//   - The two below: how many source-image pixels does one terminal
//     cell render? 2 wide x 4 tall = 8 source pixels per cell. The
//     braille rendering in `_create_img` walks a 2x4 block and assigns
//     each source pixel to one of the 8 dot positions in the braille
//     character.

/// Source-image pixels rendered per terminal cell on the horizontal axis.
pub const IMAGE_PIXELS_PER_CELL_COL: u32 = 2;

/// Source-image pixels rendered per terminal cell on the vertical axis.
pub const IMAGE_PIXELS_PER_CELL_ROW: u32 = 4;

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
        // `term_size` is documented as `(cols, rows)` in terminal cells. Each
        // cell represents an IMAGE_PIXELS_PER_CELL_COL x IMAGE_PIXELS_PER_CELL_ROW
        // pixel block (see the constants above), so we multiply accordingly
        // before resizing. We must use `resize_exact` -- the default `resize`
        // preserves aspect ratio and would shrink the long edge to fit the
        // shorter one, breaking the cell-to-pixel correspondence.
        let pixel_w = self
            .term_size
            .0
            .saturating_mul(IMAGE_PIXELS_PER_CELL_COL)
            .max(IMAGE_PIXELS_PER_CELL_COL);
        let pixel_h = self
            .term_size
            .1
            .saturating_mul(IMAGE_PIXELS_PER_CELL_ROW)
            .max(IMAGE_PIXELS_PER_CELL_ROW);
        let img = img.resize_exact(pixel_w, pixel_h, FilterType::Nearest);
        let (width, height) = img.dimensions();

        self.cloumns = width.div_ceil(IMAGE_PIXELS_PER_CELL_COL);
        self.rows = height.div_ceil(IMAGE_PIXELS_PER_CELL_ROW);
        self.picture = Some(vec![]);
        for y in (0..height).step_by(IMAGE_PIXELS_PER_CELL_ROW as usize) {
            for x in (0..width).step_by(IMAGE_PIXELS_PER_CELL_COL as usize) {
                let mut bits = 0;
                let mut r_sum = 0u32;
                let mut g_sum = 0u32;
                let mut b_sum = 0u32;
                let mut count = 0;

                for dy in 0..IMAGE_PIXELS_PER_CELL_ROW {
                    for dx in 0..IMAGE_PIXELS_PER_CELL_COL {
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

/// Decode an image into the half-block / quarter-block glyph grid and stamp
/// the cells into `term_layout` starting at `(x, y)`.
///
/// `max_size` is the `(width, height)` in terminal cells the image should be
/// resized to. Errors from the underlying loader are propagated; callers that
/// want best-effort rendering should catch and log.
pub fn render_image_at(
    term_layout: &mut TermLayout,
    path: &str,
    origin: (usize, usize),
    max_size: (u32, u32),
) -> Result<()> {
    let (x, y) = origin;
    let mut img_data = Img::new(PathBuf::from(path), max_size);
    img_data._create_img()?;
    let picture = img_data
        .picture
        .as_ref()
        .ok_or_else(|| anyhow!("Image data empty: {}", path))?;
    let img_cols = img_data.cloumns as usize;
    let img_rows = img_data.rows as usize;
    let canvas_cols = term_layout.columns as usize;
    let max = term_layout.layout.len();
    'outer: for row in 0..img_rows {
        for col in 0..img_cols {
            let canvas_idx = (y + row) * canvas_cols + (x + col);
            if canvas_idx >= max {
                break 'outer;
            }
            let pic_idx = row * img_cols + col;
            if let Some(cell) = picture.get(pic_idx) {
                term_layout.layout[canvas_idx] =
                    Block::new(cell.clone(), None, None, FontStyle::empty());
            }
        }
    }
    Ok(())
}

/// Slice-based variant of [`render_image_at`] for the parallel paint
/// pipeline. Each parallel task re-borrows its part of the canvas as a
/// `&mut [Block]`, so the image loader must be able to write through that
/// slice instead of the full `TermLayout`.
pub fn render_image_at_slice(
    layout: &mut [Block],
    canvas_cols: u16,
    path: &str,
    origin: (usize, usize),
    max_size: (u32, u32),
) -> Result<()> {
    let (x, y) = origin;
    let mut img_data = Img::new(PathBuf::from(path), max_size);
    img_data._create_img()?;
    let picture = img_data
        .picture
        .as_ref()
        .ok_or_else(|| anyhow!("Image data empty: {}", path))?;
    let img_cols = img_data.cloumns as usize;
    let img_rows = img_data.rows as usize;
    let canvas_cols = canvas_cols as usize;
    let max = layout.len();
    'outer: for row in 0..img_rows {
        for col in 0..img_cols {
            let canvas_idx = (y + row) * canvas_cols + (x + col);
            if canvas_idx >= max {
                break 'outer;
            }
            let pic_idx = row * img_cols + col;
            if let Some(cell) = picture.get(pic_idx) {
                // `picture` holds pre-styled ANSI strings (one cell each),
                // so the `word` field is the full content. Replace the
                // cell rather than trying to find a multi-byte push that
                // fits — the byte length is unbounded here.
                layout[canvas_idx] =
                    Block::new(cell.clone(), None, None, FontStyle::empty());
            }
        }
    }
    Ok(())
}

use std::{
    collections::HashMap,
    path::PathBuf,
};

use anyhow::{Result, anyhow};
use crossterm::style::Color;
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

/// Maximum number of rasterized images kept in an [`ImageCache`]. Terminal
/// resizes change the target cell size and therefore mint a new cache key
/// per size; the cap bounds memory while the common case (a handful of
/// images at one size) stays fully cached.
const MAX_CACHED_IMAGES: usize = 16;

#[derive(Debug)]
pub struct Img {
    pub path: PathBuf,
    pub term_size: (u32, u32),
    /// One [`Block`] per terminal cell. The braille glyph lives in `word`
    /// and the averaged cell color in `fcolor` — no ANSI is embedded in the
    /// text, so the paint phase can copy cells into the canvas without
    /// re-formatting, and the diff renderer compares stable cell state.
    pub picture: Option<Vec<Block>>,
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
        let mut picture = Vec::with_capacity((self.cloumns * self.rows) as usize);
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
                let mut word = String::new();
                word.push(char::from_u32(0x2800 + bits).unwrap_or(' '));
                picture.push(Block::new(word, Some(color), None, FontStyle::empty()));
            }
        }
        self.picture = Some(picture);
        Ok(())
    }
}

/// Cache of decoded + braille-rasterized images, keyed by
/// `(path, cols, rows)` in terminal cells.
///
/// The paint phase runs once per frame, and every `<img>` used to be
/// re-decoded from disk on each pass. `TermLayout` owns one of these and the
/// paint pipeline shares it across frames, so each image is decoded once per
/// size and reused for the rest of the session.
#[derive(Debug)]
pub struct ImageCache {
    entries: HashMap<(String, u32, u32), Img>,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageCache {
    /// Create an empty image cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Return the rasterized image for `(path, size)`, decoding it on first
    /// use. Failed decodes are not cached, so a temporarily missing file can
    /// recover on a later frame instead of pinning the error forever.
    pub fn get_or_decode(&mut self, path: &str, size: (u32, u32)) -> Result<&Img> {
        let key = (path.to_string(), size.0, size.1);
        if !self.entries.contains_key(&key) {
            if self.entries.len() >= MAX_CACHED_IMAGES {
                self.entries.clear();
            }
            let mut img = Img::new(PathBuf::from(path), size);
            img._create_img()?;
            self.entries.insert(key.clone(), img);
        }
        self.entries
            .get(&key)
            .ok_or_else(|| anyhow!("Image cache entry vanished for {}", path))
    }
}

/// Decode an image into the half-block / quarter-block glyph grid and stamp
/// the cells into `term_layout` starting at `(x, y)`.
///
/// `max_size` is the `(width, height)` in terminal cells the image should be
/// resized to. Errors from the underlying loader are propagated; callers that
/// want best-effort rendering should catch and log.
///
/// This one-shot entry point uses a throwaway [`ImageCache`]; long-lived
/// callers (the paint pipeline) should share a cache across frames via
/// [`render_image_at_slice`] instead.
pub fn render_image_at(
    term_layout: &mut TermLayout,
    path: &str,
    origin: (usize, usize),
    max_size: (u32, u32),
) -> Result<()> {
    let columns = term_layout.columns;
    let mut cache = ImageCache::new();
    render_image_at_slice(&mut cache, &mut term_layout.layout, columns, path, origin, max_size)
}

/// Slice-based variant of [`render_image_at`] that writes through a
/// caller-provided `&mut [Block]` instead of a full `TermLayout`.
///
/// `cache` is shared across frames so identical `(path, size)` requests hit
/// the previously decoded raster instead of re-opening the file.
pub fn render_image_at_slice(
    cache: &mut ImageCache,
    layout: &mut [Block],
    canvas_cols: u16,
    path: &str,
    origin: (usize, usize),
    max_size: (u32, u32),
) -> Result<()> {
    let (x, y) = origin;
    let img_data = cache.get_or_decode(path, max_size)?;
    let picture = img_data
        .picture
        .as_ref()
        .ok_or_else(|| anyhow!("Image data empty: {}", path))?;
    let img_cols = img_data.cloumns as usize;
    let img_rows = img_data.rows as usize;
    let canvas_cols = canvas_cols as usize;
    let max = layout.len();
    for row in 0..img_rows {
        let target_row = y + row;
        if target_row.saturating_mul(canvas_cols) >= max {
            break;
        }
        for col in 0..img_cols {
            let cx = x + col;
            if cx >= canvas_cols {
                // Clip at the right edge of the canvas row: the flat buffer
                // would otherwise wrap the overflow into the next row's cells.
                break;
            }
            let canvas_idx = target_row * canvas_cols + cx;
            let pic_idx = row * img_cols + col;
            let Some(src) = picture.get(pic_idx) else {
                continue;
            };
            let dst = &mut layout[canvas_idx];
            // Copy the rasterized cell into the canvas, reusing the canvas
            // cell's existing String allocation (no per-frame heap traffic).
            dst.word.clear();
            dst.word.push_str(&src.word);
            dst.fcolor = src.fcolor;
            dst.bcolor = src.bcolor;
            dst.style = src.style;
        }
    }
    Ok(())
}

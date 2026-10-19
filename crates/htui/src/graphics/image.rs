use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use crossterm::style::Color;
use image::imageops::FilterType;
use indexmap::IndexMap;

use super::{
    canvas::TermLayout,
    cell::{Block, FontStyle},
};
use crate::resources;

/// Errors produced while loading or rasterizing an image.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// The image source could not be resolved through the resource layer
    /// (unregistered `mem://` key).
    #[error("image load failed for {path}: {message}")]
    Load {
        /// The image path or virtual key that failed to load.
        path: String,
        /// The resource-layer error message.
        message: String,
    },
    /// The image data could not be decoded (corrupt, or unsupported format).
    #[error("image decode failed for {path}: {source}")]
    Decode {
        /// The image path that failed to decode.
        path: String,
        /// The underlying decode error.
        #[source]
        source: image::ImageError,
    },
    /// The SVG source could not be parsed or rasterized.
    #[error("svg rasterize failed for {path}: {source}")]
    SvgRasterize {
        /// The image path that failed to rasterize.
        path: String,
        /// The SVG parse / render error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A decoded image entry vanished from the cache between insertion and
    /// lookup.
    #[error("Image cache entry vanished for {path}")]
    CacheEntryVanished {
        /// The image path whose entry disappeared.
        path: String,
    },
    /// The decoded image carried no rasterized pixel data.
    #[error("Image data empty: {path}")]
    DataEmpty {
        /// The image path with empty pixel data.
        path: String,
    },
}

// ============================================================
// Image pixel -> terminal cell sampling ratio.
//
// Half-block rendering: each terminal cell displays a 1×2 block of
// source pixels. The top pixel becomes the cell's foreground color,
// the bottom pixel its background color, and the upper-half-block
// glyph U+2580 draws them, so every cell carries two real color
// samples instead of the single averaged color of the old 2×4
// braille mode. The ratio is intentionally independent of
// `crate::html::css_parse::values::PX_PER_CELL`, which governs CSS px
// in the box model — the two answer different questions:
//   - PX_PER_CELL: how many CSS px is one terminal cell? Answer: 8.
//   - The constants below: how many source-image pixels does one
//     terminal cell render? 1 wide x 2 tall.

/// Source-image pixels rendered per terminal cell on the horizontal axis.
pub const IMAGE_PIXELS_PER_CELL_COL: u32 = 1;

/// Source-image pixels rendered per terminal cell on the vertical axis.
pub const IMAGE_PIXELS_PER_CELL_ROW: u32 = 2;

/// Maximum number of rasterized images kept in an [`ImageCache`]. Terminal
/// resizes change the target cell size and therefore mint a new cache key
/// per size; the cap bounds memory while the common case (a handful of
/// images at one size) stays fully cached.
const MAX_CACHED_IMAGES: usize = 16;

/// Pick the resampling filter for a resize from `src` to `dst`:
/// `Triangle` when downscaling (each destination pixel is a weighted
/// average of nearby source pixels, avoiding the aliasing of sparse
/// nearest-neighbour sampling), `Nearest` when upscaling (keeps edges
/// crisp instead of blurring them).
fn resize_filter(src: (u32, u32), dst: (u32, u32)) -> FilterType {
    if src.0 > dst.0 || src.1 > dst.1 {
        FilterType::Triangle
    } else {
        FilterType::Nearest
    }
}

#[derive(Debug)]
pub struct Img {
    pub path: PathBuf,
    pub term_size: (u32, u32),
    /// One [`Block`] per terminal cell: the upper-half-block glyph `▀` in
    /// `word`, the top source pixel's color in `fcolor` and the bottom
    /// pixel's color in `bcolor`. No ANSI is embedded in the text, so the
    /// paint phase can copy cells into the canvas without re-formatting,
    /// and the diff renderer compares stable cell state.
    pub picture: Option<Vec<Block>>,
    pub cloumns: u32,
    pub rows: u32,
}

impl Img {
    /// Create an undecoded image for `path` targeting the given terminal size (columns, rows).
    pub fn new(path: PathBuf, term_size: (u32, u32)) -> Self {
        Self {
            path,
            term_size,
            picture: None,
            cloumns: 0,
            rows: 0,
        }
    }

    /// Decode and rasterize the source into `picture` on first use.
    pub fn _create_img(&mut self) -> Result<(), ImageError> {
        if self.picture.is_some() {
            return Ok(());
        }

        // Resolve the source through the virtual resource layer (`mem://`
        // key) and decode from the bytes.
        let src = self.path.to_string_lossy().into_owned();
        let resource = resources::load(&src).map_err(|e| ImageError::Load {
            path: src.clone(),
            message: e.to_string(),
        })?;
        let (width, height, rgba) = decode_rgba(&src, &resource)?;
        let img = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| ImageError::DataEmpty { path: src.clone() })?;
        // `term_size` is documented as `(cols, rows)` in terminal cells. Each
        // cell displays IMAGE_PIXELS_PER_CELL_COL x IMAGE_PIXELS_PER_CELL_ROW
        // source pixels (1 wide x 2 tall in half-block mode), so the pixel
        // grid is `cols x (rows * 2)`. We must resize to exactly that grid:
        // `imageops::resize` in image 0.25 scales to the exact target size
        // (the old aspect-preserving behaviour moved to `thumbnail`), keeping
        // the cell-to-pixel correspondence exact.
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
        let source = (img.width(), img.height());
        let img = image::imageops::resize(
            &img,
            pixel_w,
            pixel_h,
            resize_filter(source, (pixel_w, pixel_h)),
        );
        let (width, height) = img.dimensions();

        self.cloumns = width.div_ceil(IMAGE_PIXELS_PER_CELL_COL);
        self.rows = height.div_ceil(IMAGE_PIXELS_PER_CELL_ROW);
        let mut picture = Vec::with_capacity((self.cloumns * self.rows) as usize);
        for y in (0..height).step_by(IMAGE_PIXELS_PER_CELL_ROW as usize) {
            for x in (0..width).step_by(IMAGE_PIXELS_PER_CELL_COL as usize) {
                // height is exactly `rows * 2`, so the bottom pixel always
                // exists. The two source pixels become the cell's foreground
                // (top) and background (bottom) colors.
                let top = img.get_pixel(x, y);
                let bottom = img.get_pixel(x, y + 1);
                let fg = Color::Rgb {
                    r: top[0],
                    g: top[1],
                    b: top[2],
                };
                let bg = Color::Rgb {
                    r: bottom[0],
                    g: bottom[1],
                    b: bottom[2],
                };
                let mut word = String::new();
                word.push('\u{2580}');
                picture.push(Block::new(word, Some(fg), Some(bg), FontStyle::empty()));
            }
        }
        self.picture = Some(picture);
        Ok(())
    }
}

/// Decode `resource` into a plain (non-premultiplied) RGBA8 pixel buffer
/// `(width, height, bytes)`.
///
/// Raster formats (png / jpeg / webp) are decoded through the `image` crate;
/// when that fails and the bytes look like an SVG document, the source is
/// rasterized through `resvg` instead. The reported error is the raster
/// decoder's when the source is not an SVG, and the SVG rasterizer's when it
/// is — a corrupt PNG reports a PNG decode error, not an SVG one.
///
/// Decoding runs under explicit limits (dimension + allocation caps), so a
/// decompression bomb registered under a `mem://` key fails with a decode
/// error instead of aborting the process on a multi-gigabyte allocation.
fn decode_rgba(path: &str, resource: &[u8]) -> Result<(u32, u32, Vec<u8>), ImageError> {
    /// Largest image dimension (either axis) the decoder will accept.
    const MAX_IMAGE_DIM: u32 = 16384;
    /// Largest intermediate allocation the decoder may make while decoding.
    const MAX_IMAGE_ALLOC: u64 = 256 * 1024 * 1024;

    let decode_err = |source: image::ImageError| ImageError::Decode {
        path: path.to_string(),
        source,
    };
    let mut reader = image::ImageReader::new(std::io::Cursor::new(resource))
        .with_guessed_format()
        .map_err(|e| decode_err(image::ImageError::IoError(e)))?;
    // `Limits` is `#[non_exhaustive]`, so it is built from `Default` plus
    // field assignment instead of a struct literal.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIM);
    limits.max_image_height = Some(MAX_IMAGE_DIM);
    limits.max_alloc = Some(MAX_IMAGE_ALLOC);
    reader.limits(limits);
    match reader.decode() {
        Ok(img) => {
            let rgba = img.to_rgba8();
            Ok((rgba.width(), rgba.height(), rgba.into_raw()))
        }
        Err(raster_err) => match rasterize_svg(resource) {
            Ok((width, height, data)) => Ok((width, height, data)),
            Err(svg_err) => {
                if looks_like_svg(resource) {
                    Err(ImageError::SvgRasterize {
                        path: path.to_string(),
                        source: svg_err,
                    })
                } else {
                    Err(decode_err(raster_err))
                }
            }
        },
    }
}

/// Cheap format probe: do the leading bytes look like an SVG document?
///
/// SVG has no magic number, so we skip a UTF-8 BOM and leading whitespace and
/// check for an `<svg` root element, or the XML declaration / DOCTYPE a
/// standalone `.svg` file may carry.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || head.starts_with("<?xml") || head.starts_with("<!DOCTYPE")
}

/// Shared SVG parsing options with the system font database loaded once.
///
/// Reused across parses so text-bearing SVGs do not rescan system fonts on
/// every decode (the first decode pays the scan, the rest are cached).
fn svg_options() -> &'static resvg::usvg::Options<'static> {
    static OPTIONS: OnceLock<resvg::usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut opt = resvg::usvg::Options::default();
        opt.fontdb_mut().load_system_fonts();
        opt
    })
}

/// Rasterize an SVG document at its intrinsic size into a plain RGBA8 buffer
/// `(width, height, bytes)`.
fn rasterize_svg(
    bytes: &[u8],
) -> Result<(u32, u32, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
    use resvg::tiny_skia::Pixmap;
    use resvg::usvg::{Transform, Tree};

    let tree = Tree::from_data(bytes, svg_options())?;
    let size = tree.size();
    let width = size.width().ceil() as u32;
    let height = size.height().ceil() as u32;
    let mut pixmap = Pixmap::new(width, height).ok_or_else(|| {
        Box::<dyn std::error::Error + Send + Sync>::from(
            "svg rasterization failed: pixmap allocation too large",
        )
    })?;
    resvg::render(&tree, Transform::default(), &mut pixmap.as_mut());
    // `take_demultiplied` converts tiny-skia's premultiplied storage back to
    // straight alpha, which is what the downstream resize / half-block
    // sampling expects.
    Ok((width, height, pixmap.take_demultiplied()))
}

/// Intrinsic `(width, height)` in source pixels of an SVG document, from its
/// `width` / `height` / `viewBox` — no rasterization.
fn svg_intrinsic_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let tree = resvg::usvg::Tree::from_data(bytes, svg_options()).ok()?;
    let size = tree.size();
    Some((size.width().ceil() as u32, size.height().ceil() as u32))
}

/// Canvas-space clipping region for [`render_image_at_slice`]. Cells whose
/// canvas position falls outside the region are not stamped. `None` means
/// "no clip" — only the canvas bounds limit the stamping.
#[derive(Debug, Clone, Copy)]
pub struct ClipRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Cache of intrinsic image dimensions, so layout-time size probing parses
/// the image header at most once per source.
///
/// Entries are keyed by a hash of the *resource bytes*, not the path: the
/// registry is session-scoped and two sessions in one process may register
/// different bytes under the same path, so a path key would leak a stale
/// size across sessions (contradicting the resource layer's session
/// isolation). The map is FIFO-capped — the process-static cache can never
/// grow with the number of sessions / images.
static INTRINSIC_SIZES: OnceLock<Mutex<IndexMap<u64, (u32, u32)>>> = OnceLock::new();

/// Maximum number of cached intrinsic-size entries (see [`INTRINSIC_SIZES`]).
const MAX_INTRINSIC_SIZE_ENTRIES: usize = 64;

/// Return the intrinsic `(width, height)` in source pixels for `path`.
///
/// Resolves the source through [`resources::load`] (a `mem://` virtual key),
/// reads only the image header
/// (`image::ImageReader::into_dimensions`) — no pixel decode — and caches the
/// result per source content. SVG sources have no raster header, so their
/// intrinsic size is taken from the parsed document instead. `None` when the
/// source cannot be resolved or carries no decodable dimensions.
pub(crate) fn image_intrinsic_size(path: &str) -> Option<(u32, u32)> {
    let resource = resources::load(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    resource.as_ref().hash(&mut hasher);
    let key = hasher.finish();

    let cache = INTRINSIC_SIZES.get_or_init(|| Mutex::new(IndexMap::new()));
    let mut map = cache.lock().ok()?;
    if let Some(&size) = map.get(&key) {
        return Some(size);
    }
    let size = image::ImageReader::new(std::io::Cursor::new(resource.as_ref()))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        // SVG has no raster header to read; fall back to the document's own
        // intrinsic size.
        .or_else(|| svg_intrinsic_size(&resource));
    if let Some(size) = size {
        if map.len() >= MAX_INTRINSIC_SIZE_ENTRIES {
            map.shift_remove_index(0);
        }
        map.insert(key, size);
    }
    size
}

/// Cache of decoded + half-block-rasterized images, keyed by
/// `(path, cols, rows)` in terminal cells.
///
/// The paint phase runs once per frame, and every `<img>` used to be
/// re-decoded from disk on each pass. `TermLayout` owns one of these and the
/// paint pipeline shares it across frames, so each image is decoded once per
/// size and reused for the rest of the session. When the cap is hit the
/// oldest entry is evicted (FIFO): clearing the whole cache would force every
/// visible image to re-decode on the next frame (a visible stall whenever
/// more distinct `(path, size)` combinations than the cap appear over a
/// session, e.g. terminal resizes minting new size keys).
#[derive(Debug)]
pub struct ImageCache {
    entries: IndexMap<(String, u32, u32), Img>,
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
            entries: IndexMap::new(),
        }
    }

    /// Return the rasterized image for `(path, size)`, decoding it on first
    /// use. The source is resolved through `resources::load` (an
    /// unregistered `mem://` key is reported as an error). Failed decodes are
    /// not cached, so a temporarily unavailable source can recover on a later
    /// frame instead of pinning the error forever.
    pub fn get_or_decode(&mut self, path: &str, size: (u32, u32)) -> Result<&Img, ImageError> {
        let key = (path.to_string(), size.0, size.1);
        if !self.entries.contains_key(&key) {
            if self.entries.len() >= MAX_CACHED_IMAGES {
                self.entries.shift_remove_index(0);
            }
            let mut img = Img::new(PathBuf::from(path), size);
            img._create_img()?;
            self.entries.insert(key.clone(), img);
        }
        self.entries
            .get(&key)
            .ok_or_else(|| ImageError::CacheEntryVanished {
                path: path.to_string(),
            })
    }
}

/// Decode an image into the half-block glyph grid and stamp the cells into
/// `term_layout` starting at `(x, y)`.
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
) -> Result<(), ImageError> {
    let columns = term_layout.columns;
    let mut cache = ImageCache::new();
    render_image_at_slice(
        &mut cache,
        &mut term_layout.layout,
        columns,
        path,
        origin,
        max_size,
        None,
    )
}

/// Slice-based variant of [`render_image_at`] that writes through a
/// caller-provided `&mut [Block]` instead of a full `TermLayout`.
///
/// `cache` is shared across frames so identical `(path, size)` requests hit
/// the previously decoded raster instead of re-opening the file.
///
/// `max_size` is the image's full size in terminal cells and is always what
/// gets decoded; `clip` restricts which cells are actually stamped. Pass
/// `None` to stamp the whole image (still bounded by the canvas). This
/// decouples decode size from visibility so a partially clipped image is
/// cropped at native scale instead of being squeezed into the visible
/// region.
pub fn render_image_at_slice(
    cache: &mut ImageCache,
    layout: &mut [Block],
    canvas_cols: u16,
    path: &str,
    origin: (usize, usize),
    max_size: (u32, u32),
    clip: Option<ClipRect>,
) -> Result<(), ImageError> {
    let (x, y) = origin;
    let img_data = cache.get_or_decode(path, max_size)?;
    let picture = img_data
        .picture
        .as_ref()
        .ok_or_else(|| ImageError::DataEmpty {
            path: path.to_string(),
        })?;
    let img_cols = img_data.cloumns as usize;
    let img_rows = img_data.rows as usize;
    let canvas_cols = canvas_cols as usize;
    let max = layout.len();
    let (clip_x, clip_y, clip_w, clip_h) = match clip {
        Some(c) => (c.x, c.y, c.width, c.height),
        None => (0, 0, usize::MAX, usize::MAX),
    };
    for row in 0..img_rows {
        let target_row = y + row;
        if target_row.saturating_mul(canvas_cols) >= max {
            break;
        }
        if target_row < clip_y || target_row >= clip_y.saturating_add(clip_h) {
            continue;
        }
        for col in 0..img_cols {
            let cx = x + col;
            if cx >= canvas_cols {
                // Clip at the right edge of the canvas row: the flat buffer
                // would otherwise wrap the overflow into the next row's cells.
                break;
            }
            if cx < clip_x || cx >= clip_x.saturating_add(clip_w) {
                continue;
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

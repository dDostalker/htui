use std::{
    cell::RefCell,
    fmt::{self, Display},
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use anyhow::{Result, anyhow};
use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode},
    execute,
    style::{Color, Stylize},
};
use image::{
    GenericImageView,
    imageops::FilterType::{self},
};
use log::{debug, error, warn};
use std::io::stdout;

/// 判断字符在终端中的显示宽度（列数）
/// - ASCII / 制表符 / 窄字符: 1 列
/// - CJK / 全角字符: 2 列
pub fn char_width(c: char) -> usize {
    if c.is_ascii() {
        return 1;
    }
    let cp = c as u32;
    // 仅以下 Unicode 区块为宽字符（2 列），其余默认 1 列
    let is_wide = matches!(cp,
        // CJK 统一表意文字
        0x4E00..=0x9FFF
        | 0x3400..=0x4DBF   // CJK Ext-A
        | 0xF900..=0xFAFF   // CJK Compat Ideographs
        | 0x2F800..=0x2FA1F // CJK Compat Supplement
        // 全角形式
        | 0xFF01..=0xFF60   // Fullwidth Forms
        | 0xFFE0..=0xFFE6   // Fullwidth Signs
        // 韩文
        | 0xAC00..=0xD7AF   // Hangul Syllables
        // 中文标点 & 符号
        | 0x3000..=0x303F   // CJK Symbols/Punctuation
        | 0x2E80..=0x2EFF   // CJK Radicals
        | 0x3200..=0x33FF   // Enclosed CJK
        | 0xFFE8..=0xFFEE   // Half/Fullwidth Symbols
    );
    if is_wide { 2 } else { 1 }
}

/// block 块，终端显示的最小单元，包含每个块的内容和特性
#[derive(Debug, Clone)]
pub struct Block {
    pub(crate) word: String,
    pub(crate) fcolor: Option<Color>,
    pub(crate) bcolor: Option<Color>,
    /// 渲染后的文本
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

/// 块样式打印
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
    pub fn new_render(word: String, fcolor: Option<Color>, bcolor: Option<Color>, render: Option<String>) -> Self {
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
                debug!("render 并为染色");
                self.word.clone()
            }
        }
    }

}

/// 终端画布
#[derive(Debug, Default)]
pub struct TermLayout {
    pub layout: Vec<Block>,
    /// 当前绘画指针位置
    pub point: u32,
    /// 当前显示的字符数量
    pub len: u32,
    // 行长度
    pub columns: u16,
}

/// 画布打印
impl Display for TermLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total = self.layout.len();
        let last_row_start = total.saturating_sub(self.columns as usize);
        for (i, row) in self.layout.iter().enumerate() {
            if i >= last_row_start {
                break;
            }
            if row.word != " " {
                write!(f, "{}", row)?;
            }
        }
        write!(f, "")
    }
}

impl TermLayout {
    /// 将画布渲染到终端（含光标定位）
    /// 连续相同风格的格子合并为一次输出，减少 IO 次數、消除闪烁
    pub fn render(&self) {
        let total = self.layout.len().saturating_sub(1);
        let cols = self.columns;
        let mut i = 0;

        while i < total {
            let cell = &self.layout[i];

            // 纯空格且无背景 → 跳过
            if cell.word == " " && cell.bcolor.is_none() {
                i += 1;
                continue;
            }

            let batch_start = i;
            let fg = cell.fcolor;
            let bg = cell.bcolor;
            let mut batch = String::new();

            // 收集连续同风格格子（到行尾或风格变化为止）
            while i < total {
                let c = &self.layout[i];
                // 行边界：下一格在新行开头 → 断批
                if i > batch_start && i as u16 % cols == 0 {
                    break;
                }
                if c.word == " " && c.bcolor.is_none() {
                    break;
                }
                if c.fcolor != fg || c.bcolor != bg {
                    break;
                }
                batch.push_str(&c.word);
                i += 1;
            }

            // 定位并输出整批
            let x = batch_start as u16 % cols;
            let y = batch_start as u16 / cols;
            execute!(stdout(), MoveTo(x, y)).ok();

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

impl TermLayout {
    pub fn new_line(&mut self) {
        let columns = self.columns as u32;
        // 计算当前行还剩多少个格子
        let remaining = columns - (self.point % columns);

        self.point += remaining;
    }
    /// 替换指定画布文字（支持 CJK 宽字符）
    pub fn paint_string(&mut self, s: &str, fcolor: Option<Color>, bcolor: Option<Color>) -> u32 {
        let mut cell_count = 0u32;

        for c in s.chars() {
            let idx = self.point as usize;
            if idx >= self.layout.len() {
                break;
            }
            let width = char_width(c);
            self.layout[idx] = Block::new(c.to_string(), fcolor, bcolor);
            // 宽字符占 2 列：填充占位格，避免下一个字符覆盖第二列
            if width > 1 {
                let next_idx = idx + 1;
                if next_idx < self.layout.len() {
                    self.layout[next_idx] = Block::new(' '.to_string(), fcolor, bcolor);
                }
            }
            self.point += width as u32;
            cell_count += width as u32;
        }

        self.len += cell_count;
        cell_count
    }
    /// write_string

    /// 将图片渲染为盲文字符，写入画布
    /// - `path`：图片路径
    /// - `size`：期望缩放的像素尺寸 (max_width_px, max_height_px)
    ///  返回实际占用的画布字符总数（columns × rows）
    pub fn paint_img(&mut self, path: String, size: (u32, u32)) -> u32 {
        debug!("开始加载图片: path='{}' size={:?}", path, size);

        let mut img_data = Img::new(PathBuf::from(&path), size);
        if let Err(e) = img_data._create_img() {
            error!("路径：{}图片加载失败: {}", path, e);
            return 0;
        }
        let picture = match img_data.picture.as_ref() {
            Some(p) => p,
            None => {
                warn!("图片数据为空(可能是0尺寸或格式不支持?): {}", path);
                return 0;
            }
        };

        debug!(
            "图片加载成功: 字符尺寸 {}×{}, 占用画布字符 {}",
            img_data.cloumns,
            img_data.rows,
            picture.len()
        );

        let img_cols = img_data.cloumns as usize;
        let img_rows = img_data.rows as usize;
        let canvas_cols = self.columns as usize;

        // 图片起始行（point 当前所在行）
        let start_row = self.point as usize / canvas_cols;

        for row in 0..img_rows {
            for col in 0..img_cols {
                let canvas_idx = (start_row + row) * canvas_cols + col;
                if canvas_idx >= self.layout.len() {
                    warn!("画布越界，已截断 row={} col={}", row, col);
                    break;
                }
                let pic_idx = row * img_cols + col;
                if let Some(cell) = picture.get(pic_idx) {
                    self.layout[canvas_idx] = Block::new(cell.clone(), None, None);
                }
            }
        }

        // 将 point 移动到图片区域结束后的下一行行首
        let total_chars = (img_rows * img_cols) as u32;

        self.point = (start_row as u32 + img_rows as u32) * canvas_cols as u32;
        self.len += total_chars;

        debug!("图片加载成功，path={}，占用字符数：{}", path, total_chars);
        total_chars 
    }
}
/// 判断当前是否应该刷新
pub fn trigger_refresh() -> Option<bool> {
    // 阻塞等待事件
    match event::read() {
        Ok(Event::Key(key)) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Some(false),
            _ => {}
        },
        Ok(Event::Mouse(_)) => {
            // 鼠标事件：消费掉，不触发重绘
            return None;
        }
        Ok(Event::Resize(_, _)) => {
            // 终端大小变化：触发重绘
        }
        _ => {}
    }
    Some(true)
}
#[derive(Debug)]
struct Img {
    path: PathBuf,
    term_size: (u32, u32),
    picture: Option<Vec<String>>,
    cloumns: u32,
    rows: u32,
}

impl Img {
    /// 或许结构体得合一起比较好?
    fn new(path: PathBuf, term_size: (u32, u32)) -> Self {
        Self {
            path,
            term_size,
            picture: None,
            cloumns: 0,
            rows: 0,
        }
    }
    /// 图片转画面
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
                                //   点1(bit0) 点4(bit3)
                                //   点2(bit1) 点5(bit4)
                                //   点3(bit2) 点6(bit5)
                                //   点7(bit6) 点8(bit7)
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
                self.picture.as_mut().unwrap().push(format!(
                    "{}",
                    char::from_u32(0x2800 + bits)
                        .unwrap()
                        .to_string()
                        .with(color)
                ));
            }
        }
        Ok(())
    }
}

#[test]
fn test_create_img() {
    let mut img = Img::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_html/src/pic.png"),
        (320, 160),
    );
    img._create_img().unwrap();
    println!("{}, {}", img.cloumns, img.rows);
    for (i, row) in img.picture.as_ref().unwrap().iter().enumerate() {
        print!("{}", row);
        if (i + 1) % img.cloumns as usize == 0 {
            println!()
        }
    }
    println!();
}

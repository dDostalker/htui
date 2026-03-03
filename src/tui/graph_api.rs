use std::{fmt::Display, path::PathBuf};

use anyhow::Result;
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
use std::io::stdout;

/// 终端画布
#[derive(Debug, Default)]
pub struct TermLayout {
    pub layout: Vec<String>,
    /// 当前绘画指针位置
    pub point: u64,
    /// 当前显示的字符数量
    pub len: u64,
    pub columns: u16,
}
/// 画布打印
impl Display for TermLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, row) in self.layout.iter().enumerate() {
            if i == self.layout.len() - 1 {
                break;
            }
            let x = i as u16 % self.columns;
            let y = i as u16 / self.columns;
            if row != " " {
                execute!(stdout(), MoveTo(x, y)).ok();
                write!(f, "{}", row)?;
            }
        }
        write!(f, "")
    }
}

impl TermLayout {
    /// 替换指定画布文字
    pub fn paint_string(&mut self, s: &str, color: Option<Color>) -> u64 {
        let str_len = s.chars().count();

        let mut idx = self.point as usize;
        for c in s.chars() {
            if idx >= self.layout.len() {
                break;
            }
            let content = if let Some(color) = color {
                format!("{}", c.with(color))
            } else {
                c.to_string()
            };
            self.layout[idx] = content;
            idx += 1;
        }

        self.point += str_len as u64;
        let len = str_len as u64;
        self.len += len;
        len
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

        let img = image::open(&self.path).unwrap();
        let img = img.resize(self.term_size.0, self.term_size.1, FilterType::Nearest);
        let (width, height) = img.dimensions();
        // 使用实际缩放后的尺寸，而非 term_size（resize 是等比缩放，实际尺寸可能不同）
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
                                // Unicode 盲文点位标准布局（列优先）:
                                //   点1(bit0) 点4(bit3)
                                //   点2(bit1) 点5(bit4)
                                //   点3(bit2) 点6(bit5)
                                //   点7(bit6) 点8(bit7)
                                let bit_index = match (dx, dy) {
                                    (0, 0) => 0, // 点1
                                    (0, 1) => 1, // 点2
                                    (0, 2) => 2, // 点3
                                    (1, 0) => 3, // 点4
                                    (1, 1) => 4, // 点5
                                    (1, 2) => 5, // 点6
                                    (0, 3) => 6, // 点7
                                    (1, 3) => 7, // 点8
                                    _ => 0,
                                };
                                bits |= 1 << bit_index;
                            }
                        }
                    }
                }

                if count > 0 {
                    r_sum /= count;
                    g_sum /= count;
                    b_sum /= count;
                }
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
    let mut img = Img::new(PathBuf::from("./src/tui/test/pic.png"), (320, 160));
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

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

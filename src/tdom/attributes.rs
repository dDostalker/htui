use std::{collections::HashMap, fmt::Debug};

pub struct Attr {
    attributes: HashMap<String, String>,
}

impl Attr {
    pub fn new() -> Self {
        Attr {
            attributes: HashMap::new(),
        }
    }

    pub fn parse_from_str(attr_str: &str) -> Self {
        let mut attributes = HashMap::new();
        let mut chars = attr_str.chars().peekable();

        while let Some(ch) = chars.next() {
            // 跳过空白
            if ch.is_whitespace() {
                continue;
            }

            // 读取属性名
            let mut key = String::new();
            key.push(ch);
            while let Some(&next_ch) = chars.peek() {
                if next_ch == '=' || next_ch.is_whitespace() {
                    break;
                }
                key.push(chars.next().unwrap());
            }

            // 跳过空白
            while let Some(&next_ch) = chars.peek() {
                if next_ch.is_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }

            // 检查是否有等号
            let has_equals = matches!(chars.peek(), Some(&'='));
            if has_equals {
                // 跳过等号
                chars.next();
                // 跳过等号后的空白
                while let Some(&next_ch) = chars.peek() {
                    if next_ch.is_whitespace() {
                        chars.next();
                    } else {
                        break;
                    }
                }

                // 读取属性值
                let mut value = String::new();
                if let Some(&quote_ch) = chars.peek() {
                    if quote_ch == '"' || quote_ch == '\'' {
                        // 有引号的值
                        let quote = chars.next().unwrap();
                        while let Some(ch) = chars.next() {
                            if ch == quote {
                                break;
                            }
                            value.push(ch);
                        }
                    } else {
                        // 无引号的值（读到空格为止）
                        while let Some(&next_ch) = chars.peek() {
                            if next_ch.is_whitespace() {
                                break;
                            }
                            value.push(chars.next().unwrap());
                        }
                    }
                }

                attributes.insert(key, value);
            } else {
                // 没有等号，说明是布尔属性（如 disabled, checked）
                // 存储空字符串作为值
                attributes.insert(key, String::new());
            }
        }

        Attr { attributes }
    }

    /// 获取指定属性的值为String
    pub fn get(&self, key: &str) -> Option<&String> {
        self.attributes.get(key)
    }

    ///获取属性并尝试转换为 i32
    pub fn get_as_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.parse::<i32>().ok())
    }

    /// 将属性值解析为 `u32`。
    pub fn get_as_u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(|v| v.parse::<u32>().ok())
    }

    ///获取属性并尝试转换为 f64
    pub fn get_as_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.parse::<f64>().ok())
    }

    ///获取 HTML 布尔属性,只要属性存在就返回 Some(true)，不存在返回 None
    pub fn get_as_bool_html(&self, key: &str) -> Option<bool> {
        if self.has(key) { Some(true) } else { None }
    }

    ///获取字符串形枚举型布尔属性,解析 "true"/"false" 字符串值
    pub fn get_as_bool_string(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| match v.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            "" => Some(true), // 空字符串在某些上下文中表示 true
            _ => None,
        })
    }

    /// 数字形式布尔属性：解析值为 `"1"` / `"0"` 的属性。
    pub fn get_as_bool_number(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| match v.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        })
    }

    ///检查是否存在指定属性
    pub fn has(&self, key: &str) -> bool {
        self.attributes.contains_key(key)
    }

    ///通用属性标签
    //获取hidden属性
    pub fn get_hidden(&self) -> Option<bool> {
        self.get_as_bool_html("hidden")
    }

    //获取 draggable 属性并解析为布尔值
    pub fn get_draggable(&self) -> Option<bool> {
        self.get_as_bool_string("draggable")
    }

    //获取class属性
    pub fn get_class(&self) -> Option<Vec<String>> {
        self.get("class").map(|class_str| {
            class_str
                .split_whitespace()
                .map(|s| s.to_string())
                .collect()
        })
    }

    //检查特定class
    pub fn has_class(&self, class_name: &str) -> bool {
        if let Some(classes) = self.get_class() {
            classes.iter().any(|c| c == class_name)
        } else {
            false
        }
    }

    //获取tableindex中的整数
    pub fn get_tabindex(&self) -> Option<i32> {
        self.get_as_i32("tabindex")
    }

    //获取 id 属性
    pub fn get_id(&self) -> Option<&String> {
        self.get("id")
    }

    //获取 style 属性
    pub fn get_style(&self) -> Option<&String> {
        self.get("style")
    }

    //获取 title 属性（鼠标悬停提示文字）
    pub fn get_title(&self) -> Option<&String> {
        self.get("title")
    }

    //获取 lang 属性（语言代码，如 zh-CN）
    pub fn get_lang(&self) -> Option<&String> {
        self.get("lang")
    }

    //获取 accesskey 属性（快捷键）
    pub fn get_accesskey(&self) -> Option<&String> {
        self.get("accesskey")
    }

    //获取dir属性
    pub fn get_dir(&self) -> Option<&String> {
        self.get("dir")
    }

    ///获取contenteditable属性
    pub fn get_contenteditable(&self) -> Option<bool> {
        self.get_as_bool_string("contenteditable")
    }

    ///获取spellcheck属性
    pub fn get_spellcheck(&self) -> Option<bool> {
        self.get_as_bool_string("spellcheck")
    }

    ///获取translate属性
    pub fn get_translate(&self) -> Option<bool> {
        self.get("translate").and_then(|v| match v.as_str() {
            "yes" | "" => Some(true),
            "no" => Some(false),
            _ => None,
        })
    }

    ///特殊属性标签
    ///获取src中的属性
    pub fn get_src(&self) -> Option<&String> {
        self.get("src")
    }

    /// 获取 alt 属性
    pub fn get_alt(&self) -> Option<&String> {
        self.get("alt")
    }

    /// 判断是否为装饰性图片,即`alt=""为空
    pub fn is_decorative_img(&self) -> bool {
        matches!(self.get("alt"), Some(v) if v.is_empty())
    }

    /// width属性（像素宽度）
    pub fn get_width(&self) -> Option<u32> {
        self.get_as_u32("width")
    }

    /// height属性（像素高度）
    pub fn get_height(&self) -> Option<u32> {
        self.get_as_u32("height")
    }

    /// ismap属性,服务器端图像映射，布尔属性，必须在 `<a>` 内使用
    pub fn get_ismap(&self) -> bool {
        self.has("ismap")
    }

    /// usemap属性,客户端图像映射
    pub fn get_usemap(&self) -> Option<&String> {
        self.get("usemap")
    }

    /// srcset属性（响应式图片候选列表，原始字符串）
    /// 格式较复杂,暂时保留原始字符串
    pub fn get_srcset(&self) -> Option<&String> {
        self.get("srcset")
    }

    /// sizes`属性
    pub fn get_sizes(&self) -> Option<&String> {
        self.get("sizes")
    }

    ///获取href属性
    pub fn get_href(&self) -> Option<&String> {
        self.get("href")
    }

    ///hreflang属性
    pub fn get_hreflang(&self) -> Option<&String> {
        self.get("hreflang")
    }

    ///referrerpolicy属性
    pub fn get_referrerpolicy(&self) -> Option<&String> {
        self.get("referrerpolicy")
    }

    /// target属性
    pub fn get_target(&self) -> Option<&String> {
        self.get("target")
    }

    /// rel属性
    pub fn get_rel(&self) -> Option<&String> {
        self.get("rel")
    }

    ///download属性
    pub fn get_download(&self) -> Option<&String> {
        self.get("download")
    }

    ///role属性
    pub fn get_role(&self) -> Option<&String> {
        self.get("role")
    }
    ///aria-label属性
    pub fn get_aria_label(&self) -> Option<&String> {
        self.get("aria-label")
    }

    ///aria-labelledby属性
    pub fn get_aria_labelledby(&self) -> Option<&String> {
        self.get("aria-labelledby")
    }

    ///ol属性
    ///start树新风
    pub fn get_start(&self)->Option<i32>{
        self.get_as_i32("start")
    }

    ///reversed属性
    pub fn get_reversed(&self)->bool{
        self.has("reversed")
    }

    ///tyoe属性
    pub fn get_ol_type(&self)->Option<&String>{
        self.get("type")
    }

    ///li属性
    ///value属性
    pub fn get_li_value(&self)->Option<i32>{
        self.get_as_i32("value")
    }
    ///data-*自定义属性
    pub fn get_data(&self, key: &str) -> Option<&String> {
        self.get(&format!("data-{}", key))
    }
}
impl Debug for Attr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self.attributes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_get_width_height() {
        let attr = Attr::parse_from_str(r#"width="800" height="600""#);
        assert_eq!(attr.get_width(), Some(800));
        assert_eq!(attr.get_height(), Some(600));

        // 负数应返回 None
        let attr2 = Attr::parse_from_str(r#"width="-1""#);
        assert_eq!(attr2.get_width(), None);
    }
    #[test]
    fn test_is_decorative_img() {
        // alt="" 显式存在且为空 -> 装饰性
        let attr1 = Attr::parse_from_str(r#"src="spacer.gif" alt="""#);
        assert!(attr1.is_decorative_img());

        // alt 有内容 -> 不是装饰性
        let attr2 = Attr::parse_from_str(r#"src="photo.jpg" alt="A sunset""#);
        assert!(!attr2.is_decorative_img());

        // alt 缺失 -> 不是装饰性（是无障碍问题，但不是装饰性）
        let attr3 = Attr::parse_from_str(r#"src="photo.jpg""#);
        assert!(!attr3.is_decorative_img());
    }
    #[test]
    fn test_get_role() {
        let attr = Attr::parse_from_str(r#"role="button""#);
        assert_eq!(attr.get_role(), Some(&"button".to_string()));

        let attr2 = Attr::parse_from_str(r#"class="nav""#);
        assert_eq!(attr2.get_role(), None);
    }

    #[test]
    fn test_get_data() {
        let attr = Attr::parse_from_str(r#"data-id="123" data-type="card""#);
        assert_eq!(attr.get_data("id"), Some(&"123".to_string()));
        assert_eq!(attr.get_data("type"), Some(&"card".to_string()));

        // 不存在的 data 属性
        assert_eq!(attr.get_data("notexist"), None);
    }
    #[test]
    fn test_get_contenteditable() {
        let attr1 = Attr::parse_from_str(r#"contenteditable="true""#);
        assert_eq!(attr1.get_contenteditable(), Some(true));

        let attr2 = Attr::parse_from_str(r#"contenteditable="false""#);
        assert_eq!(attr2.get_contenteditable(), Some(false));

        // 空字符串等同 true
        let attr3 = Attr::parse_from_str(r#"contenteditable="""#);
        assert_eq!(attr3.get_contenteditable(), Some(true));

        // "inherit" 不映射为布尔
        let attr4 = Attr::parse_from_str(r#"contenteditable="inherit""#);
        assert_eq!(attr4.get_contenteditable(), None);
    }

    #[test]
    fn test_get_spellcheck_translate() {
        let attr = Attr::parse_from_str(r#"spellcheck="true" translate="no""#);
        assert_eq!(attr.get_spellcheck(), Some(true));
        assert_eq!(attr.get_translate(), Some(false));

        let attr2 = Attr::parse_from_str(r#"spellcheck="false" translate="yes""#);
        assert_eq!(attr2.get_spellcheck(), Some(false));
        assert_eq!(attr2.get_translate(), Some(true));
    }

    #[test]
    fn test_get_aria() {
        let attr = Attr::parse_from_str(r#"aria-label="关闭" aria-labelledby="title-id""#);
        assert_eq!(attr.get_aria_label(), Some(&"关闭".to_string()));
        assert_eq!(attr.get_aria_labelledby(), Some(&"title-id".to_string()));

        let attr2 = Attr::parse_from_str(r#"class="btn""#);
        assert_eq!(attr2.get_aria_label(), None);
        assert_eq!(attr2.get_aria_labelledby(), None);
    }

    #[test]
    fn test_get_a_extra() {
        let attr = Attr::parse_from_str(
            r#"href="https://example.com" hreflang="en" referrerpolicy="no-referrer""#,
        );
        assert_eq!(attr.get_hreflang(), Some(&"en".to_string()));
        assert_eq!(attr.get_referrerpolicy(), Some(&"no-referrer".to_string()));

        let attr2 = Attr::parse_from_str(r#"href="https://example.com""#);
        assert_eq!(attr2.get_hreflang(), None);
        assert_eq!(attr2.get_referrerpolicy(), None);
    }

    #[test]
    fn test_ol_attrs() {
        let attr = Attr::parse_from_str(r#"start="3" type="a""#);
        assert_eq!(attr.get_start(), Some(3));
        assert_eq!(attr.get_ol_type(), Some(&"a".to_string()));

        // reversed 是布尔属性
        let attr2 = Attr::parse_from_str(r#"reversed"#);
        assert!(attr2.get_reversed());

        let attr3 = Attr::parse_from_str(r#"start="1""#);
        assert!(!attr3.get_reversed());
    }

    #[test]
    fn test_li_value() {
        let attr = Attr::parse_from_str(r#"value="5""#);
        assert_eq!(attr.get_li_value(), Some(5));

        // 负数合法（ol 可以倒序）
        let attr2 = Attr::parse_from_str(r#"value="-1""#);
        assert_eq!(attr2.get_li_value(), Some(-1));

        let attr3 = Attr::parse_from_str(r#"class="item""#);
        assert_eq!(attr3.get_li_value(), None);
    }
}

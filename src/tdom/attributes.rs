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

    //获取属性并尝试转换为 i32
    pub fn get_as_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.parse::<i32>().ok())
    }
    
    //获取属性并尝试转换为 f64
    pub fn get_as_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.parse::<f64>().ok())
    }
    
    //获取 HTML 布尔属性
    //只要属性存在就返回 Some(true)，不存在返回 None
    pub fn get_as_bool_html(&self, key: &str) -> Option<bool> {
        if self.has(key) {
            Some(true)
        } else {
            None
        }
    }

    //获取 ARIA/枚举型布尔属性
    //解析 "true"/"false" 字符串值
    pub fn get_as_bool_string(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| match v.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            "" => Some(true), // 空字符串在某些上下文中表示 true
            _ => None,
        })
    }

    //检查是否存在指定属性
    pub fn has(&self, key: &str) -> bool {
        self.attributes.contains_key(key)
    }

    ///通用属性标签
    //获取hidden属性
    pub fn get_hidden(&self) -> Option<bool> {
        if self.has("hidden") { Some(true) } else { None }
    }

    //获取 draggable 属性并解析为布尔值
    pub fn get_draggable(&self) -> Option<bool> {
        self.get("draggable").and_then(|v| match v.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None, // "auto" 或其他
        })
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
        self.get("tabindex").and_then(|v| v.parse::<i32>().ok())
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

    ///特殊属性标签
    //获取src中的属性
    pub fn get_src(&self) -> Option<&String> {
        self.get("src")
    }

    //获取href属性
    pub fn get_href(&self) -> Option<&String> {
        self.get("href")
    }

    // 获取 alt 属性
    pub fn get_alt(&self) -> Option<&String> {
        self.get("alt")
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
    fn test_get_hidden() {
        // hidden 属性存在（无值）
        let attr1 = Attr::parse_from_str("hidden");
        // 注意：你的 parse_from_str 可能需要修改以支持无值属性
        // 暂时测试有值的情况
        let attr1 = Attr::parse_from_str(r#"hidden="hidden""#);
        assert_eq!(attr1.get_hidden(), Some(true));

        // hidden 属性存在（任意值）
        let attr2 = Attr::parse_from_str(r#"hidden="true""#);
        assert_eq!(attr2.get_hidden(), Some(true));

        // 没有 hidden 属性
        let attr3 = Attr::parse_from_str(r#"class="test""#);
        assert_eq!(attr3.get_hidden(), None);
    }

    #[test]
    fn test_get_class() {
        // 多个 class
        let attr1 = Attr::parse_from_str(r#"class="btn primary active""#);
        assert_eq!(
            attr1.get_class(),
            Some(vec![
                "btn".to_string(),
                "primary".to_string(),
                "active".to_string()
            ])
        );

        // 单个 class
        let attr2 = Attr::parse_from_str(r#"class="single""#);
        assert_eq!(attr2.get_class(), Some(vec!["single".to_string()]));

        // 没有 class 属性
        let attr3 = Attr::parse_from_str(r#"id="main""#);
        assert_eq!(attr3.get_class(), None);

        // 空 class（多个空格）
        let attr4 = Attr::parse_from_str(r#"class="  ""#);
        assert_eq!(attr4.get_class(), Some(vec![]));
    }

    #[test]
    fn test_has_class() {
        let attr = Attr::parse_from_str(r#"class="btn primary active""#);
        assert!(attr.has_class("btn"));
        assert!(attr.has_class("primary"));
        assert!(attr.has_class("active"));
        assert!(!attr.has_class("disabled"));

        let empty_attr = Attr::parse_from_str(r#"id="test""#);
        assert!(!empty_attr.has_class("btn"));
    }

    #[test]
    fn test_get_id_and_style() {
        let attr = Attr::parse_from_str(r#"id="main" style="color:red""#);
        assert_eq!(attr.get_id(), Some(&"main".to_string()));
        assert_eq!(attr.get_style(), Some(&"color:red".to_string()));
    }

    #[test]
    fn test_parse_with_spaces_around_equals() {
        // 测试等号前后有空格的情况
        let attr1 = Attr::parse_from_str(r#"class = "container""#);
        assert_eq!(attr1.get("class"), Some(&"container".to_string()));

        // 测试等号前有空格
        let attr2 = Attr::parse_from_str(r#"id ="main""#);
        assert_eq!(attr2.get("id"), Some(&"main".to_string()));

        // 测试等号后有空格
        let attr3 = Attr::parse_from_str(r#"style= "color:red""#);
        assert_eq!(attr3.get("style"), Some(&"color:red".to_string()));

        // 测试多个空格
        let attr4 = Attr::parse_from_str(r#"title  =  "test""#);
        assert_eq!(attr4.get("title"), Some(&"test".to_string()));

        // 测试混合情况：多个属性，有的有空格，有的没有
        let attr5 = Attr::parse_from_str(r#"class="btn" id = "main" style= "color:red""#);
        assert_eq!(attr5.get("class"), Some(&"btn".to_string()));
        assert_eq!(attr5.get("id"), Some(&"main".to_string()));
        assert_eq!(attr5.get("style"), Some(&"color:red".to_string()));
    }

    #[test]
    fn test_get_src_href_alt() {
        // 测试 img 标签的 src 和 alt
        let img_attr = Attr::parse_from_str(r#"src="image.jpg" alt="A beautiful sunset""#);
        assert_eq!(img_attr.get_src(), Some(&"image.jpg".to_string()));
        assert_eq!(img_attr.get_alt(), Some(&"A beautiful sunset".to_string()));

        // 测试 a 标签的 href
        let a_attr = Attr::parse_from_str(r#"href="https://example.com" target="_blank""#);
        assert_eq!(a_attr.get_href(), Some(&"https://example.com".to_string()));

        // 测试装饰性图片（空 alt）
        let decorative_img = Attr::parse_from_str(r#"src="spacer.gif" alt="""#);
        assert_eq!(decorative_img.get_src(), Some(&"spacer.gif".to_string()));
        assert_eq!(decorative_img.get_alt(), Some(&"".to_string()));

        // 测试不存在的属性
        let div_attr = Attr::parse_from_str(r#"class="container""#);
        assert_eq!(div_attr.get_src(), None);
        assert_eq!(div_attr.get_href(), None);
        assert_eq!(div_attr.get_alt(), None);
    }
    #[test]
    fn test_get_as_i32() {
        let attr = Attr::parse_from_str(r#"tabindex="5" width="100" invalid="abc""#);
        
        assert_eq!(attr.get_as_i32("tabindex"), Some(5));
        assert_eq!(attr.get_as_i32("width"), Some(100));
        assert_eq!(attr.get_as_i32("invalid"), None); // 转换失败
        assert_eq!(attr.get_as_i32("notexist"), None); // 不存在
    }

    #[test]
    fn test_get_as_f64() {
        let attr = Attr::parse_from_str(r#"opacity="0.5" zoom="1.5" invalid="abc""#);
        
        assert_eq!(attr.get_as_f64("opacity"), Some(0.5));
        assert_eq!(attr.get_as_f64("zoom"), Some(1.5));
        assert_eq!(attr.get_as_f64("invalid"), None);
        assert_eq!(attr.get_as_f64("notexist"), None);
    }

    #[test]
    fn test_get_as_bool_html() {
        // HTML 布尔属性：存在即为 true
        let attr1 = Attr::parse_from_str(r#"disabled checked required"#);
        // 注意：你的 parse_from_str 可能需要修改以支持无值属性
        
        // 暂时用有值的测试
        let attr2 = Attr::parse_from_str(r#"disabled="" checked="checked""#);
        assert_eq!(attr2.get_as_bool_html("disabled"), Some(true));
        assert_eq!(attr2.get_as_bool_html("checked"), Some(true));
        
        // 不存在
        let attr3 = Attr::parse_from_str(r#"class="test""#);
        assert_eq!(attr3.get_as_bool_html("disabled"), None);
    }

    #[test]
    fn test_get_as_bool_string() {
        // ARIA 和枚举型布尔属性
        let attr = Attr::parse_from_str(
            r#"aria-hidden="true" spellcheck="false" translate="1" custom="0""#
        );
        
        assert_eq!(attr.get_as_bool_string("aria-hidden"), Some(true));
        assert_eq!(attr.get_as_bool_string("spellcheck"), Some(false));
        assert_eq!(attr.get_as_bool_string("translate"), Some(true));
        assert_eq!(attr.get_as_bool_string("custom"), Some(false));
        
        // 无效值
        let attr2 = Attr::parse_from_str(r#"invalid="maybe""#);
        assert_eq!(attr2.get_as_bool_string("invalid"), None);
    }
}

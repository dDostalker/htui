use log::debug;
use std::{collections::HashMap, fmt::Debug};

#[derive(Clone, Default)]
pub struct Attr {
    attributes: HashMap<String, String>,
}

impl Attr {
    pub fn new() -> Self {
        Attr {
            attributes: HashMap::new(),
        }
    }

    /// Build an [`Attr`] directly from `(name, value)` pairs.
    ///
    /// Calling this is cheaper than `parse_from_str` — it avoids the
    /// round-trip `key="value" key="value"` serialization that the HTML parser
    /// used to feed into the text parser. `parse_from_str` is kept as a public
    /// helper for tests and one-off callers; the HTML parser path uses this
    /// constructor directly with `html5ever::Attribute` slices.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        let attributes = pairs.into_iter().collect();
        Attr { attributes }
    }

    pub fn parse_from_str(attr_str: &str) -> Self {
        debug!("开始解析属性字符串: {}", attr_str);

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
                        for ch in chars.by_ref() {
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

                attributes.insert(key.clone(), value.clone());
                debug!("解析到属性：key='{}' value='{}'", key, value);
            } else {
                // 没有等号，说明是布尔属性（如 disabled, checked）
                // 存储空字符串作为值
                attributes.insert(key.clone(), String::new());
                debug!("布尔属性：{}", key);
            }
        }
        debug!("属性解析完成，共{}项", attributes.len());
        Attr { attributes }
    }

    /// Get the value of an attribute as a `String`.
    pub fn get(&self, key: &str) -> Option<&String> {
        self.attributes.get(key)
    }

    ///获取属性并尝试转换为 i32
    pub fn get_as_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.parse::<i32>().ok())
    }

    /// Parse the attribute value as `u32`.
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

    /// Numeric boolean attribute: parse `"1"` / `"0"` values.
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

    /// Get the `alt` attribute.
    pub fn get_alt(&self) -> Option<&String> {
        self.get("alt")
    }

    /// Check if this is a decorative image (`alt=""`).
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

    /// Get the `srcset` attribute (responsive image candidate list, raw string).
    /// The format is complex; the raw string is preserved as-is.
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

    ///ol特殊属性
    ///start树新风
    pub fn get_start(&self) -> Option<i32> {
        self.get_as_i32("start")
    }

    ///reversed属性
    pub fn get_reversed(&self) -> bool {
        self.has("reversed")
    }

    ///tyoe属性
    pub fn get_ol_type(&self) -> Option<&String> {
        self.get("type")
    }

    ///li特殊属性
    ///value属性
    pub fn get_li_value(&self) -> Option<i32> {
        self.get_as_i32("value")
    }

    ///input特殊属性
    ///type属性
    pub fn get_input_type(&self) -> Option<&String> {
        self.get("type")
    }
    ///name属性
    pub fn get_name(&self) -> Option<&String> {
        self.get("name")
    }

    ///value属性
    pub fn get_value(&self) -> Option<&String> {
        self.get("value")
    }

    ///placeholder属性
    pub fn get_placeholder(&self) -> Option<&String> {
        self.get("placeholder")
    }

    ///disabled属性
    pub fn get_disabled(&self) -> bool {
        self.has("disabled")
    }

    ///required属性
    pub fn get_required(&self) -> bool {
        self.has("required")
    }

    ///readonly属性
    pub fn get_readonly(&self) -> bool {
        self.has("readonly")
    }

    ///autofocus属性
    pub fn get_autofocus(&self) -> bool {
        self.has("autofocus")
    }

    ///checked属性
    pub fn get_checked(&self) -> bool {
        self.has("checked")
    }

    ///multiple属性
    pub fn get_multiple(&self) -> bool {
        self.has("multiple")
    }

    ///accept属性
    pub fn get_accept(&self) -> Option<&String> {
        self.get("accept")
    }

    ///maxlength属性
    pub fn get_maxlength(&self) -> Option<u32> {
        self.get_as_u32("maxlength")
    }

    ///minlength属性
    pub fn get_minlength(&self) -> Option<u32> {
        self.get_as_u32("minlength")
    }

    ///min属性
    pub fn get_min(&self) -> Option<&String> {
        self.get("min")
    }

    ///max属性
    pub fn get_max(&self) -> Option<&String> {
        self.get("max")
    }

    ///step
    pub fn get_step(&self) -> Option<&String> {
        self.get("step")
    }

    ///pattern
    pub fn get_pattern(&self) -> Option<&String> {
        self.get("pattern")
    }

    ///autocomplete属性
    pub fn get_autocomplete(&self) -> Option<&String> {
        self.get("autocomplete")
    }

    ///list属性
    pub fn get_list(&self) -> Option<&String> {
        self.get("list")
    }

    ///form特殊属性
    ///action属性
    pub fn get_action(&self) -> Option<&String> {
        self.get("action")
    }

    ///method属性
    pub fn get_method(&self) -> Option<&String> {
        self.get("method")
    }

    ///enctype属性
    pub fn get_enctype(&self) -> Option<&String> {
        self.get("enctype")
    }

    ///target属性
    pub fn get_form_target(&self) -> Option<&String> {
        self.get("target")
    }

    ///novalidate属性
    pub fn get_novalidate(&self) -> bool {
        self.has("novalidate")
    }

    ///accept-charset属性
    pub fn get_accept_charset(&self) -> Option<&String> {
        self.get("accept-charset")
    }

    ///autocomplete属性
    pub fn get_form_autocomplete(&self) -> Option<&String> {
        self.get("autocomplete")
    }

    ///select特殊属性
    ///size属性
    pub fn get_size(&self) -> Option<u32> {
        self.get_as_u32("size")
    }

    ///form属性
    pub fn get_form_id(&self) -> Option<&String> {
        self.get("form")
    }

    ///htmloption特殊属性
    ///elected属性
    pub fn get_selected(&self) -> bool {
        self.has("selected")
    }

    ///label属性
    pub fn get_option_label(&self) -> Option<&String> {
        self.get("label")
    }

    ///button特殊属性
    ///type属性
    pub fn get_button_type(&self) -> Option<&String> {
        self.get("type")
    }

    ///textarea属性
    ///rows属性
    pub fn get_rows(&self) -> Option<u32> {
        self.get_as_u32("rows")
    }

    ///cols属性
    pub fn get_cols(&self) -> Option<u32> {
        self.get_as_u32("cols")
    }

    ///wrap属性
    pub fn get_wrap(&self) -> Option<&String> {
        self.get("wrap")
    }

    ///label特殊属性
    ///for属性
    pub fn get_label_for(&self) -> Option<&String> {
        self.get("for")
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

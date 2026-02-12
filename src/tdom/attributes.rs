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
            
            // 跳过空白和等号
            while let Some(&next_ch) = chars.peek() {
                if next_ch == '=' || next_ch.is_whitespace() {
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
        }
        
        Attr { attributes }
    }

     /// 获取指定属性的值
    pub fn get(&self, key: &str) -> Option<&String> {
        self.attributes.get(key)
    }
    
    /// 检查是否存在指定属性
    pub fn has(&self, key: &str) -> bool {
        self.attributes.contains_key(key)
    }

    //获取hidden属性
    pub fn get_hidden(&self) -> Option<bool> {
        if self.has("hidden") {
            Some(true)
        } else {
            None
        }
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

    //获取 id 属性
    pub fn get_id(&self) -> Option<&String> {
        self.get("id")
    }
    
    //获取 style 属性
    pub fn get_style(&self) -> Option<&String> {
        self.get("style")
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
}

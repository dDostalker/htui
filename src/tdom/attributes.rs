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
        
        // 按空格分割
        for attr in attr_str.split_ascii_whitespace() {
            // 按等号分割键值对
            if let Some(pos) = attr.find('=') {
                let key = &attr[..pos];
                let value = &attr[pos + 1..];
                
                // 去除引号
                let value = value.trim_matches('"').trim_matches('\'');
                
                attributes.insert(key.to_string(), value.to_string());
            }
        }
        
        Attr { attributes }
    }
}
impl Debug for Attr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self.attributes))
    }
}



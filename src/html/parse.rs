use anyhow::{Ok, Result, anyhow};

struct Parser {
    point: usize,
    context: String,
}
/// the return of element read fn
/// cmp means the element is matched
/// length means the length of element
struct RleRet {
    cmp: bool,
    length: usize,
}
/// To solve the question when we have a html like :
/// ```html
/// 1111<p>1111<\p>
/// ```
/// we should return
/// [ElementBolckRead::Str(String::from("111")),ElementBlockRead::Block(String::from("<p>1111<\p>"))]
enum ElementBlockRead {
    Str(String),
    Block(String),
}

enum ElementKind {}

///Parser machine
impl Parser {
    /// use to get next char from input context
    /// Result to duel the nth error
    /// the Option to duel the end of file
    fn _next_char(&mut self) -> Result<Option<char>> {
        if self._eof() {
            return Ok(None);
        }
        match self.context.chars().nth(self.point) {
            Some(c) => {
                self.point += c.len_utf8();
                Ok(Some(c))
            }
            None => Err(anyhow!(format!("bad char in point {}", self.point))),
        }
    }
    /// read element from content, when start with <,end with >
    fn _element_content(&mut self) -> Result<&str> {
        let start = self.point;
        let mut chars = self._next_char()?;
        todo!();
        Ok(&self.context[start..self.point])
    }
    fn _eof(&self) -> bool {
        self.point >= self.context.len()
    }
}

impl Parser {
    fn read_element<T>(&mut self, expected_fn: T) -> Result<bool>
    where
        T: Fn(&str) -> RleRet,
    {
        let ret = expected_fn();
        self.point += ret.length;
        Ok(ret.cmp)
    }
}

struct Parser {
    pos: usize,
    input: String,
}

impl Parser {
    fn _next_char(&mut self) -> Option<char> {
        self.input.chars().nth(self.pos)
    }
    fn _start_with(&mut self, s: &str) -> bool {
        self.input[self.pos..].starts_with(s)
    }
    fn _expect(&mut self, expected: &str) -> Result<(), String> {
        if self.start_with(expected) {
            self.pos += expected.len();
            Ok(())
        } else {
            Err(format!(
                "Expected '{}', found '{}'",
                expected,
                &self.input[self.pos..]
            ))
        }
    }
    fn _eof(&self) -> bool {
        self.pos >= self.input.len()
    }
}

impl Parser {
    fn next_char(&mut self) -> Option<char> {
        let ret = self._next_char();
        self.pos += ret.len_utf8();
        ret
    }
}

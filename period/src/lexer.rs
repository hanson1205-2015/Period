use crate::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Let, Set, Show, If, Then, Otherwise, While, Repeat, For, In,
    Define, With, Return, Be, To, And, Or, Not,
    Class, Init, New, Tell, The, Of, Import, From, Returns,
    // Literals
    Number(f64), String(String), Ident(String),
    // Operators
    Plus, Minus, Star, Slash, Percent, Power,
    EqEq, NotEq, Less, Greater, LessEq, GreaterEq,
    // Punctuation
    Comma, Dot, Colon, LParen, RParen, LBracket, RBracket, LBrace, RBrace,
    // Indentation
    Indent, Dedent, Newline, Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub struct Lexer<'a> {
    source: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    pending: Vec<Token>,
    at_line_start: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.char_indices().peekable(),
            line: 1,
            col: 1,
            indent_stack: vec![0],
            pending: Vec::new(),
            at_line_start: true,
        }
    }

    pub fn next_token(&mut self) -> Token {
        if let Some(t) = self.pending.pop() {
            return t;
        }
        loop {
            if let Some(kind) = self.lex_token() {
                return Token { kind, span: self.span() };
            }
            if self.peek_char().is_none() {
                break;
            }
        }
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            return Token { kind: TokenKind::Dedent, span: self.span() };
        }
        Token { kind: TokenKind::Eof, span: self.span() }
    }

    fn span(&self) -> Span { Span { line: self.line, col: self.col } }

    fn error(&self, msg: &str) -> ! {
        eprintln!("lexer error at {}:{}: {}", self.line, self.col, msg);
        std::process::exit(1);
    }

    fn peek_char(&mut self) -> Option<char> { self.chars.peek().map(|(_, c)| *c) }

    fn advance(&mut self) -> Option<char> {
        let (_, c) = self.chars.next()?;
        if c == '\n' { self.line += 1; self.col = 1; }
        else { self.col += 1; }
        Some(c)
    }

    fn lex_token(&mut self) -> Option<TokenKind> {
        if self.at_line_start {
            self.at_line_start = false;
            if self.peek_char().is_none() {
                return None;
            }
            let mut spaces = 0usize;
            while let Some(c) = self.peek_char() {
                if c == ' ' { spaces += 1; self.advance(); }
                else if c == '\t' { spaces += 4; self.advance(); }
                else { break; }
            }
            if self.peek_char() == Some('\n') {
                // blank line
                self.advance();
                self.at_line_start = true;
                return self.lex_token();
            }
            if self.peek_char() == Some('-') {
                // Could be a comment (--) or a minus operator at line start.
                // Look ahead to decide.
                let mut peeker = self.chars.clone();
                peeker.next();
                if peeker.peek().map(|(_, c)| *c) == Some('-') {
                    self.advance(); // first -
                    self.advance(); // second -
                    self.skip_comment();
                    if self.peek_char() == Some('\n') { self.advance(); }
                    self.at_line_start = true;
                    return self.lex_token();
                }
            }
            return self.handle_indent(spaces);
        }

        let c = self.peek_char()?;
        match c {
            ' ' | '\t' => { self.advance(); self.lex_token() }
            '\n' => { self.advance(); self.at_line_start = true; Some(TokenKind::Newline) }
            '-' => {
                self.advance();
                if self.peek_char() == Some('-') {
                    self.skip_comment();
                    self.lex_token()
                } else {
                    Some(TokenKind::Minus)
                }
            }
            '.' => { self.advance(); Some(TokenKind::Dot) }
            ',' => { self.advance(); Some(TokenKind::Comma) }
            ':' => { self.advance(); Some(TokenKind::Colon) }
            '(' => { self.advance(); Some(TokenKind::LParen) }
            ')' => { self.advance(); Some(TokenKind::RParen) }
            '[' => { self.advance(); Some(TokenKind::LBracket) }
            ']' => { self.advance(); Some(TokenKind::RBracket) }
            '{' => { self.advance(); Some(TokenKind::LBrace) }
            '}' => { self.advance(); Some(TokenKind::RBrace) }
            '+' => { self.advance(); Some(TokenKind::Plus) }
            '*' => {
                self.advance();
                if self.peek_char() == Some('*') { self.advance(); Some(TokenKind::Power) }
                else { Some(TokenKind::Star) }
            }
            '/' => { self.advance(); Some(TokenKind::Slash) }
            '%' => { self.advance(); Some(TokenKind::Percent) }
            '<' => {
                self.advance();
                if self.peek_char() == Some('=') { self.advance(); Some(TokenKind::LessEq) }
                else { Some(TokenKind::Less) }
            }
            '>' => {
                self.advance();
                if self.peek_char() == Some('=') { self.advance(); Some(TokenKind::GreaterEq) }
                else { Some(TokenKind::Greater) }
            }
            '=' => {
                self.advance();
                if self.peek_char() == Some('=') { self.advance(); Some(TokenKind::EqEq) }
                else { self.error("unexpected '='") }
            }
            '!' => {
                self.advance();
                if self.peek_char() == Some('=') { self.advance(); Some(TokenKind::NotEq) }
                else { self.error("unexpected '!'") }
            }
            '"' => Some(TokenKind::String(self.read_string())),
            d if d.is_ascii_digit() => Some(TokenKind::Number(self.read_number())),
            a if a.is_alphabetic() || a == '_' => {
                let name = self.read_identifier();
                let kind = match name.as_str() {
                    "let" => TokenKind::Let, "set" => TokenKind::Set, "show" => TokenKind::Show,
                    "if" => TokenKind::If, "then" => TokenKind::Then, "otherwise" => TokenKind::Otherwise,
                    "while" => TokenKind::While, "repeat" => TokenKind::Repeat,
                    "for" => TokenKind::For, "in" => TokenKind::In,
                    "define" => TokenKind::Define, "with" => TokenKind::With,
                    "return" => TokenKind::Return, "be" => TokenKind::Be, "to" => TokenKind::To,
                    "and" => TokenKind::And, "or" => TokenKind::Or, "not" => TokenKind::Not,
                    "class" => TokenKind::Class, "init" => TokenKind::Init,
                    "new" => TokenKind::New, "tell" => TokenKind::Tell,
                    "the" => TokenKind::The, "of" => TokenKind::Of,
                    "import" => TokenKind::Import, "from" => TokenKind::From,
                    "returns" => TokenKind::Returns,
                    "true" => TokenKind::Number(1.0),
                    "false" => TokenKind::Number(0.0),
                    "nothing" => TokenKind::Number(0.0),
                    _ => TokenKind::Ident(name),
                };
                Some(kind)
            }
            _ => self.error(&format!("unexpected character '{}'", c)),
        }
    }

    fn handle_indent(&mut self, spaces: usize) -> Option<TokenKind> {
        let top = *self.indent_stack.last().unwrap();
        if spaces > top {
            self.indent_stack.push(spaces);
            Some(TokenKind::Indent)
        } else if spaces < top {
            while spaces < *self.indent_stack.last().unwrap() {
                self.indent_stack.pop();
                self.pending.push(Token { kind: TokenKind::Dedent, span: self.span() });
            }
            if spaces != *self.indent_stack.last().unwrap() {
                self.error("inconsistent indentation");
            }
            self.pending.pop().map(|t| t.kind).or_else(|| self.lex_token())
        } else {
            self.lex_token()
        }
    }

    fn skip_comment(&mut self) {
        while let Some(c) = self.peek_char() {
            if c == '\n' { break; }
            self.advance();
        }
    }

    fn read_string(&mut self) -> String {
        self.advance(); // opening quote
        let mut result = String::new();
        while let Some(c) = self.peek_char() {
            if c == '"' { self.advance(); break; }
            if c == '\\' {
                self.advance();
                match self.advance() {
                    Some('n') => result.push('\n'),
                    Some('t') => result.push('\t'),
                    Some('"') => result.push('"'),
                    Some('\\') => result.push('\\'),
                    Some(other) => result.push(other),
                    None => self.error("unterminated string"),
                }
            } else {
                result.push(c);
                self.advance();
            }
        }
        result
    }

    fn read_number(&mut self) -> f64 {
        let start = self.col;
        let mut saw_dot = false;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() || c == '_' { self.advance(); }
            else if c == '.' && !saw_dot {
                // Make sure it's a decimal point, not statement terminator at EOL
                if let Some((_, next)) = self.chars.clone().next() {
                    if next.is_ascii_digit() {
                        saw_dot = true;
                        self.advance();
                        continue;
                    }
                }
                break;
            }
            else { break; }
        }
        let end = self.col;
        let line = self.source.lines().nth(self.line - 1).unwrap();
        let text: String = line[start - 1..end - 1].chars().filter(|c| *c != '_').collect();
        text.parse().unwrap_or_else(|_| self.error("invalid number"))
    }

    fn read_identifier(&mut self) -> String {
        let start_line = self.line;
        let start_col = self.col;
        let mut chars = Vec::new();
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                chars.push(c);
                self.advance();
            } else { break; }
        }
        if self.line == start_line {
            let line = self.source.lines().nth(start_line - 1).unwrap();
            line[start_col - 1..self.col - 1].to_string()
        } else {
            chars.into_iter().collect()
        }
    }
}

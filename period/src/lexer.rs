use crate::ast::Span;
use num_bigint::BigInt;

#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
    Literal(String),
    Expr(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Let, Set, Show, If, Then, Otherwise, While, Repeat, For, In,
    Define, With, Return, Be, To, And, Or, Not,
    Class, Init, New, Tell, The, Of, Import, From, Returns,
    Read, Write, Try, Catch, Export,
    // Literals
    Integer(BigInt), Number(f64), String(String), Interpolated(Vec<StringPart>), Ident(String), Bool(bool), Nothing,
    // Operators
    Plus, Minus, Star, Slash, Percent, Power,
    EqEq, NotEq, Less, Greater, LessEq, GreaterEq,
    // Punctuation
    Comma, Dot, Ellipsis, Colon, LParen, RParen, LBracket, RBracket, LBrace, RBrace,
    // Indentation
    Indent, Dedent, Newline, Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub struct Lexer<'a> {
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
            chars: source.char_indices().peekable(),
            line: 1,
            col: 1,
            indent_stack: vec![0],
            pending: Vec::new(),
            at_line_start: true,
        }
    }

    pub fn next_token(&mut self) -> Result<Token, String> {
        if let Some(t) = self.pending.pop() {
            return Ok(t);
        }
        loop {
            let start = self.span();
            if let Some(kind) = self.lex_token()? {
                return Ok(Token { kind, span: start });
            }
            if self.peek_char().is_none() {
                break;
            }
        }
        if self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            return Ok(Token { kind: TokenKind::Dedent, span: self.span() });
        }
        Ok(Token { kind: TokenKind::Eof, span: self.span() })
    }

    fn span(&self) -> Span { Span { line: self.line, col: self.col } }

    fn error(&self, msg: &str) -> String {
        format!("lexer error at {}:{}: {}", self.line, self.col, msg)
    }

    fn peek_char(&mut self) -> Option<char> { self.chars.peek().map(|(_, c)| *c) }

    fn peek_char_at(&mut self, offset: usize) -> Option<char> {
        self.chars.clone().nth(offset).map(|(_, c)| c)
    }

    fn advance(&mut self) -> Option<char> {
        let (_, c) = self.chars.next()?;
        if c == '\n' { self.line += 1; self.col = 1; }
        else { self.col += 1; }
        Some(c)
    }

    fn lex_token(&mut self) -> Result<Option<TokenKind>, String> {
        if self.at_line_start {
            self.at_line_start = false;
            if self.peek_char().is_none() {
                return Ok(None);
            }
            let mut spaces = 0usize;
            while let Some(c) = self.peek_char() {
                if c == ' ' { spaces += 1; self.advance(); }
                else if c == '\t' { spaces += 4; self.advance(); }
                else { break; }
            }
            if self.peek_char() == Some('\r') {
                self.advance();
                if self.peek_char() == Some('\n') {
                    self.advance();
                }
                self.at_line_start = true;
                return self.lex_token();
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

        let c = match self.peek_char() {
            Some(c) => c,
            None => return Ok(None),
        };
        match c {
            ' ' | '\t' | '\r' => { self.advance(); Ok(None) }
            '\n' => { self.advance(); self.at_line_start = true; Ok(Some(TokenKind::Newline)) }
            '-' => {
                self.advance();
                if self.peek_char() == Some('-') {
                    self.skip_comment();
                    self.lex_token()
                } else {
                    Ok(Some(TokenKind::Minus))
                }
            }
            '.' => {
                self.advance();
                // Relative import paths: ./foo or ../foo. Read the whole path as one identifier.
                if self.peek_char() == Some('/') {
                    let mut path = String::from("./");
                    self.advance(); // consume '/'
                    while let Some(c) = self.peek_char() {
                        if c.is_alphanumeric() || c == '_' || c == '/' { path.push(c); self.advance(); }
                        else { break; }
                    }
                    Ok(Some(TokenKind::Ident(path)))
                } else if self.peek_char() == Some('.') {
                    self.advance();
                    if self.peek_char() == Some('/') {
                        let mut path = String::from("../");
                        self.advance(); // consume '/'
                        while let Some(c) = self.peek_char() {
                            if c.is_alphanumeric() || c == '_' || c == '/' { path.push(c); self.advance(); }
                            else { break; }
                        }
                        Ok(Some(TokenKind::Ident(path)))
                    } else if self.peek_char() == Some('.') {
                        self.advance();
                        Ok(Some(TokenKind::Ellipsis))
                    } else {
                        Err(self.error("unexpected '..'"))
                    }
                } else {
                    Ok(Some(TokenKind::Dot))
                }
            }
            ',' => { self.advance(); Ok(Some(TokenKind::Comma)) }
            ':' => { self.advance(); Ok(Some(TokenKind::Colon)) }
            '(' => { self.advance(); Ok(Some(TokenKind::LParen)) }
            ')' => { self.advance(); Ok(Some(TokenKind::RParen)) }
            '[' => { self.advance(); Ok(Some(TokenKind::LBracket)) }
            ']' => { self.advance(); Ok(Some(TokenKind::RBracket)) }
            '{' => { self.advance(); Ok(Some(TokenKind::LBrace)) }
            '}' => { self.advance(); Ok(Some(TokenKind::RBrace)) }
            '+' => { self.advance(); Ok(Some(TokenKind::Plus)) }
            '*' => {
                self.advance();
                if self.peek_char() == Some('*') { self.advance(); Ok(Some(TokenKind::Power)) }
                else { Ok(Some(TokenKind::Star)) }
            }
            '/' => { self.advance(); Ok(Some(TokenKind::Slash)) }
            '%' => { self.advance(); Ok(Some(TokenKind::Percent)) }
            '<' => {
                self.advance();
                if self.peek_char() == Some('=') { self.advance(); Ok(Some(TokenKind::LessEq)) }
                else { Ok(Some(TokenKind::Less)) }
            }
            '>' => {
                self.advance();
                if self.peek_char() == Some('=') { self.advance(); Ok(Some(TokenKind::GreaterEq)) }
                else { Ok(Some(TokenKind::Greater)) }
            }
            '=' => {
                self.advance();
                if self.peek_char() == Some('=') { self.advance(); Ok(Some(TokenKind::EqEq)) }
                else { Err(self.error("unexpected '='")) }
            }
            '!' => {
                self.advance();
                if self.peek_char() == Some('=') { self.advance(); Ok(Some(TokenKind::NotEq)) }
                else { Err(self.error("unexpected '!'")) }
            }
            '"' | '\'' => Ok(Some(self.read_string_token()?)),
            d if d.is_ascii_digit() => Ok(Some(self.read_number()?)),
            a if a.is_alphabetic() || a == '_' => {
                let name = self.read_identifier();
                let lower = name.to_ascii_lowercase();
                // Keywords are case-insensitive: Let, LET, let all mean the same thing.
                // Identifiers (variables, functions, classes) keep their original case.
                let kind = match lower.as_str() {
                    "let" => TokenKind::Let, "set" => TokenKind::Set, "show" => TokenKind::Show,
                    "if" => TokenKind::If, "then" => TokenKind::Then, "otherwise" | "else" => TokenKind::Otherwise,
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
                    "read" => TokenKind::Read, "write" => TokenKind::Write,
                    "try" => TokenKind::Try, "catch" => TokenKind::Catch,
                    "export" => TokenKind::Export,
                    "true" => TokenKind::Bool(true),
                    "false" => TokenKind::Bool(false),
                    "nothing" => TokenKind::Nothing,
                    _ => TokenKind::Ident(name),
                };
                Ok(Some(kind))
            }
            _ => Err(self.error(&format!("unexpected character '{}'", c))),
        }
    }

    fn handle_indent(&mut self, spaces: usize) -> Result<Option<TokenKind>, String> {
        let top = *self.indent_stack.last().unwrap_or(&0);
        if spaces > top {
            self.indent_stack.push(spaces);
            Ok(Some(TokenKind::Indent))
        } else if spaces < top {
            while spaces < *self.indent_stack.last().unwrap_or(&0) {
                self.indent_stack.pop();
                self.pending.push(Token { kind: TokenKind::Dedent, span: self.span() });
            }
            if spaces != *self.indent_stack.last().unwrap_or(&0) {
                return Err(self.error("inconsistent indentation"));
            }
            self.pending.pop().map(|t| Ok(Some(t.kind))).unwrap_or_else(|| self.lex_token())
        } else {
            Ok(None)
        }
    }

    fn skip_comment(&mut self) {
        while let Some(c) = self.peek_char() {
            if c == '\n' { break; }
            self.advance();
        }
    }

    fn read_string_token(&mut self) -> Result<TokenKind, String> {
        // Both "double" and 'single' quotes delimit strings, with identical
        // semantics including interpolation.
        let quote = self.advance().unwrap_or('"');
        let mut literal = String::new();
        let mut parts: Vec<StringPart> = Vec::new();
        let mut has_interp = false;

        fn flush(literal: &mut String, parts: &mut Vec<StringPart>) {
            if !literal.is_empty() {
                parts.push(StringPart::Literal(std::mem::take(literal)));
            }
        }

        let mut closed = false;
        while let Some(c) = self.peek_char() {
            if c == quote { self.advance(); closed = true; break; }
            if c == '\\' {
                self.advance();
                match self.advance() {
                    Some('n') => literal.push('\n'),
                    Some('t') => literal.push('\t'),
                    Some('"') => literal.push('"'),
                    Some('\'') => literal.push('\''),
                    Some('\\') => literal.push('\\'),
                    Some('{') => literal.push('{'),
                    Some('}') => literal.push('}'),
                    Some(other) => literal.push(other),
                    None => return Err(self.error("unterminated string")),
                }
            } else if c == '{' {
                if self.peek_char_at(1) == Some('{') {
                    return Err(self.error("unexpected '{{'; use \\{ to escape a literal brace"));
                }
                has_interp = true;
                flush(&mut literal, &mut parts);
                self.advance(); // '{'
                let expr = self.read_interpolation_expr(quote)?;
                parts.push(StringPart::Expr(expr));
            } else {
                literal.push(c);
                self.advance();
            }
        }

        if !closed {
            return Err(self.error("unterminated string"));
        }

        if has_interp {
            flush(&mut literal, &mut parts);
            Ok(TokenKind::Interpolated(parts))
        } else {
            Ok(TokenKind::String(literal))
        }
    }

    fn read_interpolation_expr(&mut self, quote: char) -> Result<String, String> {
        let mut expr = String::new();
        let mut depth = 1;
        while let Some(c) = self.peek_char() {
            if c == quote { return Err(self.error("unterminated interpolation expression")); }
            if c == '{' {
                depth += 1;
                expr.push(c);
                self.advance();
            } else if c == '}' {
                depth -= 1;
                self.advance();
                if depth == 0 { break; }
                expr.push(c);
            } else if c == '\\' {
                self.advance();
                if let Some(escaped) = self.advance() {
                    expr.push('\\');
                    expr.push(escaped);
                }
            } else {
                expr.push(c);
                self.advance();
            }
        }
        Ok(expr)
    }

    pub fn lex_string(source: &str) -> Result<Vec<Token>, String> {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let t = lexer.next_token()?;
            let eof = matches!(t.kind, TokenKind::Eof);
            tokens.push(t);
            if eof { break; }
        }
        Ok(tokens)
    }

    fn read_number(&mut self) -> Result<TokenKind, String> {
        let mut chars = Vec::new();
        let mut saw_dot = false;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() || c == '_' {
                chars.push(c);
                self.advance();
            }
            else if c == '.' && !saw_dot {
                // Make sure the character after the dot is a digit (decimal point)
                // rather than the end of the number / statement terminator.
                if let Some((_, next)) = self.chars.clone().nth(1)
                    && next.is_ascii_digit() {
                        saw_dot = true;
                        chars.push(c);
                        self.advance();
                        continue;
                    }
                break;
            }
            else { break; }
        }
        // Collect chars instead of slicing the line: columns count characters,
        // not bytes, so byte-slicing breaks on multi-byte characters.
        let text: String = chars.into_iter().filter(|c| *c != '_').collect();
        if saw_dot {
            match text.parse() {
                Ok(n) => Ok(TokenKind::Number(n)),
                Err(_) => Err(self.error("invalid number")),
            }
        } else {
            match text.parse::<BigInt>() {
                Ok(n) => Ok(TokenKind::Integer(n)),
                Err(_) => Err(self.error("invalid number")),
            }
        }
    }

    fn read_identifier(&mut self) -> String {
        let mut chars = Vec::new();
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                chars.push(c);
                self.advance();
            } else { break; }
        }
        chars.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<TokenKind> {
        let mut lexer = Lexer::new(source);
        let mut out = Vec::new();
        loop {
            let t = lexer.next_token().expect("lexer should produce a token");
            let eof = matches!(t.kind, TokenKind::Eof);
            out.push(t.kind);
            if eof { break; }
        }
        out
    }

    #[test]
    fn tokenize_let_statement() {
        assert_eq!(
            tokens("let x be 10."),
            vec![
                TokenKind::Let,
                TokenKind::Ident("x".to_string()),
                TokenKind::Be,
                TokenKind::Integer(BigInt::from(10)),
                TokenKind::Dot,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn tokenize_string_literal() {
        let toks = tokens("show \"hello\".");
        assert!(matches!(toks[1], TokenKind::String(ref s) if s == "hello"));
    }

    #[test]
    fn tokenize_single_quoted_string() {
        let toks = tokens("show 'hello'.");
        assert!(matches!(toks[1], TokenKind::String(ref s) if s == "hello"));

        // Double quotes need no escaping inside single-quoted strings.
        let toks = tokens("show 'say \"hi\"'.");
        assert!(matches!(toks[1], TokenKind::String(ref s) if s == "say \"hi\""));

        // Escaped single quote.
        let toks = tokens("show 'it\\'s'.");
        assert!(matches!(toks[1], TokenKind::String(ref s) if s == "it's"));
    }

    #[test]
    fn integer_vs_number_literals() {
        let toks = tokens("5 5.0");
        assert!(matches!(toks[0], TokenKind::Integer(ref n) if *n == BigInt::from(5)));
        assert!(matches!(toks[1], TokenKind::Number(n) if n == 5.0));
    }

    #[test]
    fn lex_error_unterminated_string() {
        let mut lexer = Lexer::new("\"hello");
        assert!(lexer.next_token().is_err());
    }

    #[test]
    fn same_indent_lines_keep_correct_spans() {
        fn spans(source: &str) -> Vec<Span> {
            let mut lexer = Lexer::new(source);
            let mut out = Vec::new();
            loop {
                let t = lexer.next_token().expect("lexer should produce a token");
                let eof = matches!(t.kind, TokenKind::Eof);
                out.push(t.span);
                if eof { break; }
            }
            out
        }
        let source = "show a.\nshow ab.\nshow abc.";
        let s = spans(source);
        // Token stream: show, a, ., Newline, show, ab, ., Newline, show, abc, ., Eof
        // Identifiers are at indices 1, 5, 9.
        assert_eq!(s[1], Span { line: 1, col: 6 });
        assert_eq!(s[5], Span { line: 2, col: 6 });
        assert_eq!(s[9], Span { line: 3, col: 6 });
    }
}

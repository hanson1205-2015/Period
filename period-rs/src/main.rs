use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::process::Command;

// =========================================================================
// Lexer
// =========================================================================
#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    // Keywords
    Let, Set, Show, If, Then, Otherwise, While, Repeat, For, In,
    Define, With, Return, Be, To, And, Or, Not,
    // Literals
    Number(f64), Ident(String),
    // Operators
    Plus, Minus, Star, Slash, Percent, Power,
    EqEq, NotEq, Less, Greater, LessEq, GreaterEq,
    // Punctuation
    Comma, Dot, Colon, LParen, RParen,
    // Indentation
    Indent, Dedent, Newline, Eof,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    line: usize,
    col: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    span: Span,
}

struct Lexer<'a> {
    source: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    pending: Vec<Token>,
    at_line_start: bool,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
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

    fn peek_char(&mut self) -> Option<char> {
        self.chars.peek().map(|(_, c)| *c)
    }

    fn advance(&mut self) -> Option<char> {
        let (_, c) = self.chars.next()?;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn span(&self) -> Span {
        Span { line: self.line, col: self.col }
    }

    fn error(&self, msg: &str) -> ! {
        eprintln!("lexer error at {}:{}: {}", self.line, self.col, msg);
        std::process::exit(1);
    }

    fn push(&mut self, kind: TokenKind) {
        self.pending.push(Token { kind, span: self.span() });
    }

    fn handle_indent(&mut self, spaces: usize) -> Option<TokenKind> {
        let top = *self.indent_stack.last().unwrap();
        if spaces > top {
            self.indent_stack.push(spaces);
            Some(TokenKind::Indent)
        } else if spaces < top {
            while spaces < *self.indent_stack.last().unwrap() {
                self.indent_stack.pop();
                self.push(TokenKind::Dedent);
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

    fn read_number(&mut self) -> f64 {
        let start = self.col;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() || c == '_' { self.advance(); } else { break; }
        }
        let end = self.col;
        let text: String = self.source.lines().nth(self.line - 1).unwrap()[start - 1..end - 1].chars().filter(|c| *c != '_').collect();
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
            } else {
                break;
            }
        }
        // Use source slice if on same line.
        if self.line == start_line {
            self.source.lines().nth(start_line - 1).unwrap()[start_col - 1..self.col - 1].to_string()
        } else {
            chars.into_iter().collect()
        }
    }

    fn lex_token(&mut self) -> Option<TokenKind> {
        if self.at_line_start {
            self.at_line_start = false;
            let mut spaces = 0usize;
            while let Some(c) = self.peek_char() {
                if c == ' ' { spaces += 1; self.advance(); }
                else if c == '\t' { spaces += 4; self.advance(); }
                else { break; }
            }
            if self.peek_char() == Some('\n') || self.peek_char() == Some('#') {
                // blank/comment line: reset to line start after handling
                return self.lex_token();
            }
            return self.handle_indent(spaces);
        }

        let c = self.peek_char()?;
        match c {
            ' ' | '\t' => { self.advance(); self.lex_token() }
            '\n' => { self.advance(); self.at_line_start = true; Some(TokenKind::Newline) }
            '#' => { self.skip_comment(); self.lex_token() }
            '.' => { self.advance(); Some(TokenKind::Dot) }
            ',' => { self.advance(); Some(TokenKind::Comma) }
            ':' => { self.advance(); Some(TokenKind::Colon) }
            '(' => { self.advance(); Some(TokenKind::LParen) }
            ')' => { self.advance(); Some(TokenKind::RParen) }
            '+' => { self.advance(); Some(TokenKind::Plus) }
            '-' => {
                self.advance();
                if self.peek_char() == Some('-') {
                    self.skip_comment();
                    self.lex_token()
                } else {
                    Some(TokenKind::Minus)
                }
            }
            '*' => {
                self.advance();
                if self.peek_char() == Some('*') {
                    self.advance();
                    Some(TokenKind::Power)
                } else {
                    Some(TokenKind::Star)
                }
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
            d if d.is_ascii_digit() => Some(TokenKind::Number(self.read_number())),
            a if a.is_alphabetic() || a == '_' => {
                let name = self.read_identifier();
                let kind = match name.as_str() {
                    "let" => TokenKind::Let,
                    "set" => TokenKind::Set,
                    "show" => TokenKind::Show,
                    "if" => TokenKind::If,
                    "then" => TokenKind::Then,
                    "otherwise" => TokenKind::Otherwise,
                    "while" => TokenKind::While,
                    "repeat" => TokenKind::Repeat,
                    "for" => TokenKind::For,
                    "in" => TokenKind::In,
                    "define" => TokenKind::Define,
                    "with" => TokenKind::With,
                    "return" => TokenKind::Return,
                    "be" => TokenKind::Be,
                    "to" => TokenKind::To,
                    "and" => TokenKind::And,
                    "or" => TokenKind::Or,
                    "not" => TokenKind::Not,
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

    fn next_token(&mut self) -> Token {
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
        // Emit dedents and EOF.
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            return Token { kind: TokenKind::Dedent, span: self.span() };
        }
        Token { kind: TokenKind::Eof, span: self.span() }
    }
}

// =========================================================================
// AST
// =========================================================================
#[derive(Debug, Clone)]
enum Expr {
    Number(f64),
    Var(String),
    Binary { op: String, left: Box<Expr>, right: Box<Expr> },
    Unary { op: String, operand: Box<Expr> },
    Call { callee: String, args: Vec<Expr> },
}

#[derive(Debug, Clone)]
enum Stmt {
    Let { name: String, value: Expr },
    Set { name: String, value: Expr },
    Show(Expr),
    If { cond: Expr, then_branch: Vec<Stmt>, else_branch: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    For { var: String, stop: Expr, body: Vec<Stmt> },
    Return(Option<Expr>),
    Define { name: String, params: Vec<String>, body: Vec<Stmt> },
}

#[derive(Debug)]
struct Program {
    statements: Vec<Stmt>,
}

// =========================================================================
// Parser
// =========================================================================
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self, offset: usize) -> &Token {
        &self.tokens[self.pos + offset]
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() { self.pos += 1; }
        t
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek(0).kind) == std::mem::discriminant(kind)
    }

    fn expect(&mut self, kind: TokenKind, msg: &str) -> Token {
        if self.check(&kind) { self.advance() }
        else {
            let t = self.peek(0);
            eprintln!("parse error at {}:{}: {}", t.span.line, t.span.col, msg);
            std::process::exit(1);
        }
    }

    fn skip_newlines(&mut self) {
        while self.check(&TokenKind::Newline) { self.advance(); }
    }

    fn parse_program(&mut self) -> Program {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.check(&TokenKind::Eof) {
            stmts.push(self.parse_statement());
            self.skip_newlines();
        }
        Program { statements: stmts }
    }

    fn parse_statement(&mut self) -> Stmt {
        self.skip_newlines();
        match self.peek(0).kind {
            TokenKind::Let => self.parse_let(),
            TokenKind::Set => self.parse_set(),
            TokenKind::Show => self.parse_show(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Define => self.parse_define(),
            TokenKind::Return => self.parse_return(),
            _ => self.parse_expr_statement(),
        }
    }

    fn parse_let(&mut self) -> Stmt {
        self.advance(); // let
        let name = self.expect_ident("expected variable name");
        self.expect(TokenKind::Be, "expected 'be' after variable name");
        let value = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of let");
        Stmt::Let { name, value }
    }

    fn parse_set(&mut self) -> Stmt {
        self.advance(); // set
        let name = self.expect_ident("expected variable name");
        self.expect(TokenKind::To, "expected 'to' in set");
        let value = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of set");
        Stmt::Set { name, value }
    }

    fn parse_show(&mut self) -> Stmt {
        self.advance(); // show
        let expr = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of show");
        Stmt::Show(expr)
    }

    fn parse_if(&mut self) -> Stmt {
        self.advance(); // if
        let cond = self.parse_expression();
        self.expect(TokenKind::Then, "expected 'then' after if condition");
        self.expect(TokenKind::Colon, "expected ':' after then");
        let then_branch = self.parse_block();
        let mut else_branch = Vec::new();
        self.skip_newlines();
        if self.check(&TokenKind::Otherwise) {
            self.advance();
            self.expect(TokenKind::Colon, "expected ':' after otherwise");
            else_branch = self.parse_block();
        }
        Stmt::If { cond, then_branch, else_branch }
    }

    fn parse_while(&mut self) -> Stmt {
        self.advance(); // while
        let cond = self.parse_expression();
        self.expect(TokenKind::Repeat, "expected 'repeat' after while condition");
        self.expect(TokenKind::Colon, "expected ':' after repeat");
        let body = self.parse_block();
        Stmt::While { cond, body }
    }

    fn parse_for(&mut self) -> Stmt {
        self.advance(); // for
        let var = self.expect_ident("expected loop variable");
        self.expect(TokenKind::In, "expected 'in' in for");
        let callee = self.expect_ident("expected 'range'");
        if callee != "range" {
            eprintln!("only 'range' is supported in for loops");
            std::process::exit(1);
        }
        self.expect(TokenKind::With, "expected 'with' after range");
        let stop = self.parse_expression();
        self.expect(TokenKind::Repeat, "expected 'repeat' after range");
        self.expect(TokenKind::Colon, "expected ':' after repeat");
        let body = self.parse_block();
        Stmt::For { var, stop, body }
    }

    fn parse_define(&mut self) -> Stmt {
        self.advance(); // define
        let name = self.expect_ident("expected function name");
        let mut params = Vec::new();
        if self.check(&TokenKind::With) {
            self.advance();
            loop {
                params.push(self.expect_ident("expected parameter name"));
                if !self.check(&TokenKind::Comma) { break; }
                self.advance();
            }
        }
        self.expect(TokenKind::Colon, "expected ':' after function signature");
        let body = self.parse_block();
        Stmt::Define { name, params, body }
    }

    fn parse_return(&mut self) -> Stmt {
        self.advance(); // return
        let value = if self.check(&TokenKind::Dot) { None } else { Some(self.parse_expression()) };
        self.expect(TokenKind::Dot, "expected '.' at end of return");
        Stmt::Return(value)
    }

    fn parse_expr_statement(&mut self) -> Stmt {
        let expr = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of expression statement");
        Stmt::Show(expr)
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        self.skip_newlines();
        self.expect(TokenKind::Indent, "expected indented block");
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(&TokenKind::Dedent) || self.check(&TokenKind::Eof) { break; }
            stmts.push(self.parse_statement());
        }
        if self.check(&TokenKind::Dedent) { self.advance(); }
        stmts
    }

    fn expect_ident(&mut self, msg: &str) -> String {
        if let TokenKind::Ident(name) = self.peek(0).kind.clone() {
            self.advance();
            name
        } else {
            let t = self.peek(0);
            eprintln!("parse error at {}:{}: {}", t.span.line, t.span.col, msg);
            std::process::exit(1);
        }
    }

    fn parse_expression(&mut self) -> Expr {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        while self.check(&TokenKind::Or) {
            self.advance();
            let right = self.parse_and();
            left = Expr::Binary { op: "||".to_string(), left: Box::new(left), right: Box::new(right) };
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_equality();
        while self.check(&TokenKind::And) {
            self.advance();
            let right = self.parse_equality();
            left = Expr::Binary { op: "&&".to_string(), left: Box::new(left), right: Box::new(right) };
        }
        left
    }

    fn parse_equality(&mut self) -> Expr {
        let mut left = self.parse_comparison();
        loop {
            let op = if self.check(&TokenKind::EqEq) { "==" }
            else if self.check(&TokenKind::NotEq) { "!=" }
            else { break; };
            self.advance();
            let right = self.parse_comparison();
            left = Expr::Binary { op: op.to_string(), left: Box::new(left), right: Box::new(right) };
        }
        left
    }

    fn parse_comparison(&mut self) -> Expr {
        let mut left = self.parse_additive();
        loop {
            let op = if self.check(&TokenKind::Less) { "<" }
            else if self.check(&TokenKind::Greater) { ">" }
            else if self.check(&TokenKind::LessEq) { "<=" }
            else if self.check(&TokenKind::GreaterEq) { ">=" }
            else { break; };
            self.advance();
            let right = self.parse_additive();
            left = Expr::Binary { op: op.to_string(), left: Box::new(left), right: Box::new(right) };
        }
        left
    }

    fn parse_additive(&mut self) -> Expr {
        let mut left = self.parse_multiplicative();
        loop {
            let op = if self.check(&TokenKind::Plus) { "+" }
            else if self.check(&TokenKind::Minus) { "-" }
            else { break; };
            self.advance();
            let right = self.parse_multiplicative();
            left = Expr::Binary { op: op.to_string(), left: Box::new(left), right: Box::new(right) };
        }
        left
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut left = self.parse_power();
        loop {
            let op = if self.check(&TokenKind::Star) { "*" }
            else if self.check(&TokenKind::Slash) { "/" }
            else if self.check(&TokenKind::Percent) { "%" }
            else { break; };
            self.advance();
            let right = self.parse_power();
            left = Expr::Binary { op: op.to_string(), left: Box::new(left), right: Box::new(right) };
        }
        left
    }

    fn parse_power(&mut self) -> Expr {
        let left = self.parse_unary();
        if self.check(&TokenKind::Power) {
            self.advance();
            let right = self.parse_power();
            Expr::Binary { op: "pow".to_string(), left: Box::new(left), right: Box::new(right) }
        } else {
            left
        }
    }

    fn parse_unary(&mut self) -> Expr {
        if self.check(&TokenKind::Minus) {
            self.advance();
            Expr::Unary { op: "-".to_string(), operand: Box::new(self.parse_unary()) }
        } else if self.check(&TokenKind::Not) {
            self.advance();
            Expr::Unary { op: "!".to_string(), operand: Box::new(self.parse_unary()) }
        } else {
            self.parse_call()
        }
    }

    fn parse_call(&mut self) -> Expr {
        if let TokenKind::Ident(name) = self.peek(0).kind.clone() {
            if self.peek(1).kind == TokenKind::With {
                self.advance(); // ident
                self.advance(); // with
                let mut args = Vec::new();
                if !self.check(&TokenKind::Dot) && !self.check(&TokenKind::Newline) && !self.check(&TokenKind::Indent) {
                    args.push(self.parse_expression());
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        args.push(self.parse_expression());
                    }
                }
                return Expr::Call { callee: name, args };
            }
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek(0).kind.clone() {
            TokenKind::Number(n) => { self.advance(); Expr::Number(n) }
            TokenKind::Ident(name) => { self.advance(); Expr::Var(name) }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression();
                self.expect(TokenKind::RParen, "expected ')'");
                expr
            }
            _ => {
                let t = self.peek(0);
                eprintln!("parse error at {}:{}: expected expression", t.span.line, t.span.col);
                std::process::exit(1);
            }
        }
    }
}

// =========================================================================
// Codegen to Rust
// =========================================================================
struct Codegen {
    output: String,
    indent: usize,
}

impl Codegen {
    fn new() -> Self {
        Self { output: String::new(), indent: 0 }
    }

    fn line(&mut self, s: &str) {
        for _ in 0..self.indent { self.output.push_str("    "); }
        self.output.push_str(s);
        self.output.push('\n');
    }

    fn gen_program(&mut self, program: &Program) {
        self.line("use std::io::Write;");
        self.line("");
        // First emit function definitions.
        for stmt in &program.statements {
            if let Stmt::Define { name, params, body } = stmt {
                self.gen_define(name, params, body);
                self.line("");
            }
        }
        // Then main.
        self.line("fn main() {");
        self.indent += 1;
        for stmt in &program.statements {
            if !matches!(stmt, Stmt::Define { .. }) {
                self.gen_stmt(stmt);
            }
        }
        self.indent -= 1;
        self.line("}");
    }

    fn gen_define(&mut self, name: &str, params: &[String], body: &[Stmt]) {
        let params_str = params.iter().map(|p| format!("mut {}: i64", p)).collect::<Vec<_>>().join(", ");
        self.line(&format!("fn {}({}) -> i64 {{", name, params_str));
        self.indent += 1;
        for stmt in body { self.gen_stmt(stmt); }
        self.line("0");
        self.indent -= 1;
        self.line("}");
    }

    fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, value } => {
                self.line(&format!("let mut {} = {};", name, self.gen_expr(value)));
            }
            Stmt::Set { name, value } => {
                self.line(&format!("{} = {};", name, self.gen_expr(value)));
            }
            Stmt::Show(expr) => {
                self.line(&format!("println!(\"{{}}\", {});", self.gen_expr(expr)));
            }
            Stmt::If { cond, then_branch, else_branch } => {
                self.line(&format!("if {} {{", self.gen_expr(cond)));
                self.indent += 1;
                for s in then_branch { self.gen_stmt(s); }
                self.indent -= 1;
                if else_branch.is_empty() {
                    self.line("}");
                } else {
                    self.line("} else {");
                    self.indent += 1;
                    for s in else_branch { self.gen_stmt(s); }
                    self.indent -= 1;
                    self.line("}");
                }
            }
            Stmt::While { cond, body } => {
                self.line(&format!("while {} {{", self.gen_expr(cond)));
                self.indent += 1;
                for s in body { self.gen_stmt(s); }
                self.indent -= 1;
                self.line("}");
            }
            Stmt::For { var, stop, body } => {
                self.line(&format!("for mut {} in 0i64..{} {{", var, self.gen_expr(stop)));
                self.indent += 1;
                for s in body { self.gen_stmt(s); }
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Return(Some(expr)) => {
                self.line(&format!("return {};", self.gen_expr(expr)));
            }
            Stmt::Return(None) => {
                self.line("return 0;");
            }
            Stmt::Define { .. } => {}
        }
    }

    fn gen_expr(&self, expr: &Expr) -> String {
        match expr {
            Expr::Number(n) => format!("{}i64", *n as i64),
            Expr::Var(name) => name.clone(),
            Expr::Binary { op, left, right } => {
                let l = self.gen_expr(left);
                let r = self.gen_expr(right);
                if op == "pow" {
                    format!("{}.pow({} as u32)", l, r)
                } else {
                    format!("({} {} {})", l, op, r)
                }
            }
            Expr::Unary { op, operand } => {
                format!("({}{})", op, self.gen_expr(operand))
            }
            Expr::Call { callee, args } => {
                let args_str = args.iter().map(|a| self.gen_expr(a)).collect::<Vec<_>>().join(", ");
                format!("{}({})", callee, args_str)
            }
        }
    }
}

// =========================================================================
// Driver
// =========================================================================
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: period-rs <file.period>");
        std::process::exit(1);
    }
    let path = &args[1];
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", path, e);
        std::process::exit(1);
    });

    let mut lexer = Lexer::new(&source);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token();
        let eof = t.kind == TokenKind::Eof;
        tokens.push(t);
        if eof { break; }
    }

    let mut parser = Parser::new(tokens);
    let program = parser.parse_program();

    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let source_hash = hasher.finish();

    let cache_dir = env::temp_dir().join("period_rs_cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let exe_path = cache_dir.join(format!("period_{:016x}.exe", source_hash));

    if !exe_path.exists() {
        let mut codegen = Codegen::new();
        codegen.gen_program(&program);
        let rust_source = codegen.output;

        let rs_path = cache_dir.join(format!("period_{:016x}.rs", source_hash));
        fs::write(&rs_path, &rust_source).unwrap();

        let rustc_path = r"C:\Users\kylez\.cargo\bin\rustc.exe";
        let rustc_status = Command::new(rustc_path)
            .arg("-C").arg("opt-level=3")
            .arg("-C").arg("target-cpu=native")
            .arg("-o").arg(&exe_path)
            .arg(&rs_path)
            .status()
            .unwrap_or_else(|e| {
                eprintln!("failed to invoke rustc: {}", e);
                std::process::exit(1);
            });
        if !rustc_status.success() {
            eprintln!("rustc failed");
            std::process::exit(1);
        }
    }

    let run_status = Command::new(&exe_path).status().unwrap_or_else(|e| {
        eprintln!("failed to run executable: {}", e);
        std::process::exit(1);
    });
    std::process::exit(run_status.code().unwrap_or(1));
}

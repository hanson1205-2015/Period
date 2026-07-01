use crate::ast::*;
use crate::lexer::{StringPart, Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self { Self { tokens, pos: 0 } }

    pub fn parse_program(&mut self) -> Program {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.check(&TokenKind::Eof) {
            stmts.push(self.parse_statement());
            self.skip_newlines();
        }
        Program { statements: stmts }
    }

    fn peek(&self, offset: usize) -> &Token {
        &self.tokens[self.pos + offset.min(self.tokens.len() - 1 - self.pos)]
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
        else { self.error(msg) }
    }

    fn error(&self, msg: &str) -> ! {
        let t = self.peek(0);
        panic!("parse error at {}:{}: {}", t.span.line, t.span.col, msg);
    }

    fn skip_newlines(&mut self) {
        while self.check(&TokenKind::Newline) { self.advance(); }
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
            TokenKind::Return => self.parse_return(),
            TokenKind::Define => self.parse_define(),
            TokenKind::Init => self.parse_init(),
            TokenKind::Class => self.parse_class(),
            TokenKind::Import => self.parse_import(),
            TokenKind::Read => self.parse_read(),
            TokenKind::Write => self.parse_write(),
            TokenKind::Try => self.parse_try(),
            TokenKind::Export => self.parse_export(),
            TokenKind::Ellipsis => { self.advance(); Stmt::Pass }
            _ => self.parse_expr_statement(),
        }
    }

    fn parse_let(&mut self) -> Stmt {
        self.advance(); // let
        let (type_ann, name) = self.parse_typed_name();
        self.expect(TokenKind::Be, "expected 'be' after variable name");
        let _type_ann = type_ann.or_else(|| self.try_parse_type_before_value());
        let value = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of let");
        Stmt::Let { name, value }
    }

    fn try_parse_type_before_value(&mut self) -> Option<String> {
        let start = self.pos;
        if let TokenKind::Ident(ref first) = self.peek(0).kind.clone() {
            if self.is_type_start(first) {
                let t = self.parse_type();
                if self.is_value_start() {
                    return Some(t);
                }
            }
        }
        self.pos = start;
        None
    }

    fn is_value_start(&self) -> bool {
        matches!(
            self.peek(0).kind,
            TokenKind::Number(_)
                | TokenKind::String(_)
                | TokenKind::Bool(_)
                | TokenKind::Nothing
                | TokenKind::Ident(_)
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::New
                | TokenKind::Tell
                | TokenKind::Minus
                | TokenKind::Not
                | TokenKind::The
                | TokenKind::Ellipsis
        )
    }

    fn parse_set(&mut self) -> Stmt {
        self.advance(); // set
        let target = self.parse_assign_target();
        self.expect(TokenKind::To, "expected 'to' in set");
        let value = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of set");
        Stmt::Set { target, value }
    }

    fn parse_assign_target(&mut self) -> AssignTarget {
        let expr = self.parse_expression();
        match expr {
            Expr::Variable { name, span } => AssignTarget::Variable { name, span },
            Expr::Index { object, index } => AssignTarget::Index { object, index },
            Expr::Property { object, name } => AssignTarget::Property { object, name },
            _ => self.error("invalid assignment target"),
        }
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
        if self.check(&TokenKind::Comma) { self.advance(); }
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
        let iterable = self.parse_expression();
        self.expect(TokenKind::Repeat, "expected 'repeat' after for iterable");
        self.expect(TokenKind::Colon, "expected ':' after repeat");
        let body = self.parse_block();
        Stmt::For { var, iterable, body }
    }

    fn parse_return(&mut self) -> Stmt {
        self.advance(); // return
        let value = if self.check(&TokenKind::Dot) { None } else { Some(self.parse_expression()) };
        self.expect(TokenKind::Dot, "expected '.' at end of return");
        Stmt::Return(value)
    }

    fn parse_define(&mut self) -> Stmt {
        self.advance(); // define
        let name = self.expect_ident("expected function name");
        let params = if self.check(&TokenKind::With) {
            self.advance();
            self.parse_params()
        } else { Vec::new() };
        let return_type = if self.check(&TokenKind::Returns) {
            self.advance();
            Some(self.parse_type())
        } else { None };
        self.expect(TokenKind::Colon, "expected ':' after function signature");
        let raw_body = self.parse_block();
        let (docstring, body) = self.strip_docstring(raw_body);
        Stmt::Define { name, params, return_type, docstring, body }
    }

    fn strip_docstring(&self, mut stmts: Vec<Stmt>) -> (Option<String>, Vec<Stmt>) {
        let mut docstring = None;
        while let Some(Stmt::Expr(Expr::String(s))) = stmts.first() {
            docstring = Some(s.clone());
            stmts.remove(0);
            break;
        }
        (docstring, stmts)
    }

    fn parse_init(&mut self) -> Stmt {
        self.advance(); // init
        let params = if self.check(&TokenKind::With) {
            self.advance();
            self.parse_params()
        } else { Vec::new() };
        self.expect(TokenKind::Colon, "expected ':' after init signature");
        let raw_body = self.parse_block();
        let (docstring, body) = self.strip_docstring(raw_body);
        Stmt::Init(Init { params, body, docstring })
    }

    fn parse_class(&mut self) -> Stmt {
        self.advance(); // class
        let name = self.expect_ident("expected class name");
        self.expect(TokenKind::Colon, "expected ':' after class name");
        let raw_members = self.parse_block();
        let (docstring, members) = self.strip_docstring(raw_members);
        let mut init = None;
        let mut methods = Vec::new();
        for m in members {
            match m {
                Stmt::Init(i) => {
                    init = Some(i);
                }
                Stmt::Define { .. } => methods.push(m),
                _ => self.error_with("class body may only contain init and methods", &m),
            }
        }
        Stmt::Class { name, init, methods, docstring }
    }

    fn parse_import(&mut self) -> Stmt {
        self.advance(); // import
        let mut paths = Vec::new();
        loop {
            paths.push(self.parse_module_path());
            if self.check(&TokenKind::Comma) { self.advance(); continue; }
            if self.check(&TokenKind::And) { self.advance(); continue; }
            break;
        }
        self.expect(TokenKind::Dot, "expected '.' at end of import");
        Stmt::Import(paths)
    }

    fn parse_read(&mut self) -> Stmt {
        self.advance(); // read
        let name = self.expect_ident("expected variable name after read");
        self.expect(TokenKind::From, "expected 'from' after variable name in read");
        let path = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of read");
        Stmt::Read { name, path }
    }

    fn parse_write(&mut self) -> Stmt {
        self.advance(); // write
        let content = self.parse_expression();
        self.expect(TokenKind::To, "expected 'to' after content in write");
        let path = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of write");
        Stmt::Write { content, path }
    }

    fn parse_try(&mut self) -> Stmt {
        self.advance(); // try
        self.expect(TokenKind::Colon, "expected ':' after try");
        let body = self.parse_block();
        self.skip_newlines();
        self.expect(TokenKind::Catch, "expected 'catch' after try block");
        let catch_var = self.expect_ident("expected variable name after catch");
        self.expect(TokenKind::Colon, "expected ':' after catch variable");
        let catch_body = self.parse_block();
        Stmt::Try { body, catch_var, catch_body }
    }

    fn parse_export(&mut self) -> Stmt {
        self.advance(); // export
        let mut names = Vec::new();
        loop {
            names.push(self.expect_ident("expected name to export"));
            if !self.check(&TokenKind::Comma) && !self.check(&TokenKind::And) { break; }
            self.advance();
        }
        self.expect(TokenKind::Dot, "expected '.' at end of export");
        Stmt::Export(names)
    }

    fn parse_module_path(&mut self) -> (String, Span) {
        let start = self.peek(0).span.clone();
        let mut dots = String::new();
        while self.check(&TokenKind::Dot) {
            dots.push('.');
            self.advance();
        }
        let mut parts = vec![self.expect_ident("expected module name")];
        while self.check(&TokenKind::Dot) && matches!(self.peek(1).kind, TokenKind::Ident(_)) {
            self.advance();
            parts.push(self.expect_ident("expected module name"));
        }
        let path = format!("{}{}", dots, parts.join("."));
        (path, start)
    }

    fn parse_expr_statement(&mut self) -> Stmt {
        let expr = self.parse_expression();
        self.expect(TokenKind::Dot, "expected '.' at end of expression statement");
        Stmt::Expr(expr)
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        self.skip_newlines();
        self.expect(TokenKind::Indent, "expected indented block");
        let mut stmts = Vec::new();
        // A leading string literal that is not followed by '.' is treated as a
        // docstring, allowing stub/interface files to write:
        //   define f with x:
        //       "doc"
        //       ...
        self.skip_newlines();
        if !self.check(&TokenKind::Dedent) && !self.check(&TokenKind::Eof) {
            if let TokenKind::String(s) = &self.peek(0).kind {
                if !matches!(self.peek(1).kind, TokenKind::Dot) {
                    let s = s.clone();
                    self.advance();
                    stmts.push(Stmt::Expr(Expr::String(s)));
                }
            }
        }
        loop {
            self.skip_newlines();
            if self.check(&TokenKind::Dedent) || self.check(&TokenKind::Eof) { break; }
            stmts.push(self.parse_statement());
        }
        if self.check(&TokenKind::Dedent) { self.advance(); }
        stmts
    }

    fn parse_params(&mut self) -> Vec<(String, Option<String>)> {
        let mut params = Vec::new();
        loop {
            let (type_ann, name) = self.parse_typed_name();
            params.push((name, type_ann));
            if !self.check(&TokenKind::Comma) { break; }
            self.advance();
        }
        params
    }

    fn is_type_start(&self, name: &str) -> bool {
        matches!(
            name,
            "nothing" | "boolean" | "integer" | "number" | "string" | "list" | "dictionary"
                | "function" | "class"
        ) || name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
    }

    fn parse_type_from(&mut self, name: String) -> String {
        match name.as_str() {
            "list" => {
                self.expect(TokenKind::Of, "expected 'of' after list");
                let elem = self.parse_type();
                format!("list of {}", elem)
            }
            "dictionary" => {
                self.expect(TokenKind::Of, "expected 'of' after dictionary");
                let key = self.parse_type();
                self.expect(TokenKind::To, "expected 'to' after dictionary key type");
                let value = self.parse_type();
                format!("dictionary of {} to {}", key, value)
            }
            _ => name,
        }
    }

    fn parse_type(&mut self) -> String {
        let name = self.expect_ident("expected type name");
        self.parse_type_from(name)
    }

    fn parse_typed_name(&mut self) -> (Option<String>, String) {
        let first = self.expect_ident("expected name");
        if self.is_type_start(&first) {
            let type_ann = self.parse_type_from(first);
            let name = self.expect_ident("expected variable name after type");
            (Some(type_ann), name)
        } else {
            (None, first)
        }
    }

    fn expect_ident(&mut self, msg: &str) -> String {
        if let TokenKind::Ident(name) = self.peek(0).kind.clone() {
            self.advance();
            name
        } else { self.error(msg) }
    }

    fn error_with(&self, msg: &str, stmt: &Stmt) -> ! {
        panic!("parse error: {}: {:?}", msg, stmt);
    }

    // Expressions ---------------------------------------------------------
    pub fn parse_expression(&mut self) -> Expr { self.parse_or() }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        while self.check(&TokenKind::Or) {
            let span = self.peek(0).span.clone();
            self.advance();
            let right = self.parse_and();
            left = Expr::Binary { op: BinOp::Or, left: Box::new(left), right: Box::new(right), span };
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_equality();
        while self.check(&TokenKind::And) {
            let span = self.peek(0).span.clone();
            self.advance();
            let right = self.parse_equality();
            left = Expr::Binary { op: BinOp::And, left: Box::new(left), right: Box::new(right), span };
        }
        left
    }

    fn parse_equality(&mut self) -> Expr {
        let mut left = self.parse_comparison();
        loop {
            let (op, span) = if self.check(&TokenKind::EqEq) { (BinOp::Eq, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::NotEq) { (BinOp::Ne, self.peek(0).span.clone()) }
            else { break; };
            self.advance();
            let right = self.parse_comparison();
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span };
        }
        left
    }

    fn parse_comparison(&mut self) -> Expr {
        let mut left = self.parse_additive();
        loop {
            let (op, span) = if self.check(&TokenKind::Less) { (BinOp::Lt, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::Greater) { (BinOp::Gt, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::LessEq) { (BinOp::Le, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::GreaterEq) { (BinOp::Ge, self.peek(0).span.clone()) }
            else { break; };
            self.advance();
            let right = self.parse_additive();
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span };
        }
        left
    }

    fn parse_additive(&mut self) -> Expr {
        let mut left = self.parse_multiplicative();
        loop {
            let (op, span) = if self.check(&TokenKind::Plus) { (BinOp::Add, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::Minus) { (BinOp::Sub, self.peek(0).span.clone()) }
            else { break; };
            self.advance();
            let right = self.parse_multiplicative();
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span };
        }
        left
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut left = self.parse_power();
        loop {
            let (op, span) = if self.check(&TokenKind::Star) { (BinOp::Mul, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::Slash) { (BinOp::Div, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::Percent) { (BinOp::Mod, self.peek(0).span.clone()) }
            else { break; };
            self.advance();
            let right = self.parse_power();
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span };
        }
        left
    }

    fn parse_power(&mut self) -> Expr {
        let left = self.parse_unary();
        if self.check(&TokenKind::Power) {
            let span = self.peek(0).span.clone();
            self.advance();
            let right = self.parse_power();
            Expr::Binary { op: BinOp::Pow, left: Box::new(left), right: Box::new(right), span }
        } else { left }
    }

    fn parse_unary(&mut self) -> Expr {
        if self.check(&TokenKind::Minus) {
            self.advance();
            Expr::Unary { op: UnaryOp::Neg, operand: Box::new(self.parse_unary()) }
        } else if self.check(&TokenKind::Not) {
            self.advance();
            Expr::Unary { op: UnaryOp::Not, operand: Box::new(self.parse_unary()) }
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        loop {
            if self.check(&TokenKind::With) {
                self.advance();
                let mut args = Vec::new();
                if !self.is_call_terminator() {
                    args.push(self.parse_expression());
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        args.push(self.parse_expression());
                    }
                }
                expr = Expr::Call { callee: Box::new(expr), args };
            } else if self.check(&TokenKind::LBracket) {
                self.advance();
                let index = self.parse_expression();
                self.expect(TokenKind::RBracket, "expected ']'");
                expr = Expr::Index { object: Box::new(expr), index: Box::new(index) };
            } else if self.check(&TokenKind::From) {
                // qualified reference: name from module
                self.advance();
                let module = self.parse_module_path().0;
                if let Expr::Variable { name, .. } = expr {
                    expr = Expr::Qualified { name, module };
                } else {
                    self.error("qualified reference must start with a name");
                }
            } else if matches!(self.peek(0).kind, TokenKind::Ident(_)) && !self.check(&TokenKind::With) {
                // property access: obj name
                // Be careful not to consume a variable in expression context as property
                let name = self.expect_ident("expected property name");
                expr = Expr::Property { object: Box::new(expr), name };
            } else if self.check(&TokenKind::Dot) && matches!(self.peek(1).kind, TokenKind::Ident(_)) {
                // property access with dot: obj.name
                self.advance(); // '.'
                let name = self.expect_ident("expected property name after '.'");
                expr = Expr::Property { object: Box::new(expr), name };
            } else {
                break;
            }
        }
        expr
    }

    fn is_call_terminator(&self) -> bool {
        self.check(&TokenKind::Dot) || self.check(&TokenKind::Newline) || self.check(&TokenKind::Indent) || self.check(&TokenKind::Dedent) || self.check(&TokenKind::Eof)
    }

    fn parse_interpolated(&self, parts: Vec<StringPart>) -> Expr {
        let mut exprs: Vec<Expr> = Vec::new();
        for part in parts {
            match part {
                StringPart::Literal(s) => exprs.push(Expr::String(s)),
                StringPart::Expr(src) => {
                    let tokens = crate::lexer::Lexer::lex_string(&src);
                    let inner = Parser::new(tokens).parse_expression();
                    // Wrap the interpolated expression with the built-in `string` function so
                    // numbers, booleans, lists, etc. are automatically converted to text.
                    let span = Span { line: 1, col: 1 };
                    let string_call = Expr::Call {
                        callee: Box::new(Expr::Variable { name: "string".to_string(), span: span.clone() }),
                        args: vec![inner],
                    };
                    exprs.push(string_call);
                }
            }
        }
        if exprs.is_empty() {
            return Expr::String(String::new());
        }
        let mut result = exprs.remove(0);
        for e in exprs {
            let span = Span { line: 1, col: 1 };
            result = Expr::Binary { op: BinOp::Add, left: Box::new(result), right: Box::new(e), span };
        }
        result
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek(0).kind.clone() {
            TokenKind::Number(n) => { self.advance(); Expr::Number(n) }
            TokenKind::String(s) => { self.advance(); Expr::String(s) }
            TokenKind::Interpolated(parts) => { self.advance(); self.parse_interpolated(parts) }
            TokenKind::Bool(b) => { self.advance(); Expr::Bool(b) }
            TokenKind::Nothing => { self.advance(); Expr::Nothing }
            TokenKind::Ellipsis => { self.advance(); Expr::Ellipsis }
            TokenKind::Ident(name) => {
                let span = self.peek(0).span.clone();
                self.advance();
                Expr::Variable { name, span }
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression();
                self.expect(TokenKind::RParen, "expected ')'");
                expr
            }
            TokenKind::LBracket => {
                self.advance();
                let mut elems = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    elems.push(self.parse_expression());
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        elems.push(self.parse_expression());
                    }
                }
                self.expect(TokenKind::RBracket, "expected ']'");
                Expr::List(elems)
            }
            TokenKind::LBrace => {
                self.advance();
                let mut pairs = Vec::new();
                if !self.check(&TokenKind::RBrace) {
                    let key = self.parse_expression();
                    self.expect(TokenKind::Colon, "expected ':' after dict key");
                    let value = self.parse_expression();
                    pairs.push((key, value));
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        let key = self.parse_expression();
                        self.expect(TokenKind::Colon, "expected ':' after dict key");
                        let value = self.parse_expression();
                        pairs.push((key, value));
                    }
                }
                self.expect(TokenKind::RBrace, "expected '}'");
                Expr::Dict(pairs)
            }
            TokenKind::New => {
                self.advance();
                let class = self.parse_primary();
                Expr::New { class: Box::new(class), args: Vec::new() }
            }
            TokenKind::Tell => {
                self.advance();
                let object = self.parse_expression();
                self.expect(TokenKind::To, "expected 'to' after tell object");
                let method = self.expect_ident("expected method name");
                let mut args = Vec::new();
                if self.check(&TokenKind::With) {
                    self.advance();
                    if !self.is_call_terminator() {
                        args.push(self.parse_expression());
                        while self.check(&TokenKind::Comma) {
                            self.advance();
                            args.push(self.parse_expression());
                        }
                    }
                }
                Expr::Tell { object: Box::new(object), method, args }
            }
            TokenKind::The => {
                self.advance();
                let name = self.expect_ident("expected property name");
                self.expect(TokenKind::Of, "expected 'of' after property name");
                let object = self.parse_expression();
                Expr::Property { object: Box::new(object), name }
            }
            _ => self.error("expected expression"),
        }
    }
}

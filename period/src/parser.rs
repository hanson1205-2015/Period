use crate::ast::*;
use crate::lexer::{StringPart, Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self { Self { tokens, pos: 0 } }

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.check(&TokenKind::Eof) {
            stmts.push(self.parse_statement()?);
            self.skip_newlines();
        }
        Ok(Program { statements: stmts })
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

    fn expect(&mut self, kind: TokenKind, msg: &str) -> Result<Token, String> {
        if self.check(&kind) { Ok(self.advance()) }
        else { Err(self.error(msg)) }
    }

    fn error(&self, msg: &str) -> String {
        let t = self.peek(0);
        format!("parse error at {}:{}: {}", t.span.line, t.span.col, msg)
    }

    fn skip_newlines(&mut self) {
        while self.check(&TokenKind::Newline) { self.advance(); }
    }

    fn parse_statement(&mut self) -> Result<Stmt, String> {
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
            TokenKind::Ellipsis => { self.advance(); Ok(Stmt::Pass) }
            _ => self.parse_expr_statement(),
        }
    }

    fn parse_let(&mut self) -> Result<Stmt, String> {
        let start = self.peek(0).span.clone();
        self.advance(); // let
        let (type_ann, name) = self.parse_typed_name()?;
        self.expect(TokenKind::Be, "expected 'be' after variable name")?;
        let type_ann = if type_ann.is_some() {
            type_ann
        } else {
            self.try_parse_type_before_value()?
        };
        let value = self.parse_expression()?;
        self.expect(TokenKind::Dot, "expected '.' at end of let")?;
        Ok(Stmt::Let { name, type_ann, value, span: start })
    }

    fn try_parse_type_before_value(&mut self) -> Result<Option<String>, String> {
        let start = self.pos;
        if let TokenKind::Ident(ref first) = self.peek(0).kind.clone()
            && self.is_type_start(first) {
                let t = self.parse_type()?;
                if self.is_value_start() {
                    return Ok(Some(t));
                }
            }
        self.pos = start;
        Ok(None)
    }

    fn is_value_start(&self) -> bool {
        matches!(
            self.peek(0).kind,
            TokenKind::Integer(_)
                | TokenKind::Number(_)
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

    fn parse_set(&mut self) -> Result<Stmt, String> {
        self.advance(); // set
        let target = self.parse_assign_target()?;
        self.expect(TokenKind::To, "expected 'to' in set")?;
        let value = self.parse_expression()?;
        self.expect(TokenKind::Dot, "expected '.' at end of set")?;
        Ok(Stmt::Set { target, value })
    }

    fn parse_assign_target(&mut self) -> Result<AssignTarget, String> {
        let expr = self.parse_expression()?;
        match expr {
            Expr::Variable { name, span } => Ok(AssignTarget::Variable { name, span }),
            Expr::Index { object, index, span } => Ok(AssignTarget::Index { object, index, span }),
            Expr::Property { object, name, span } => Ok(AssignTarget::Property { object, name, span }),
            _ => Err(self.error("invalid assignment target")),
        }
    }

    fn parse_show(&mut self) -> Result<Stmt, String> {
        self.advance(); // show
        let expr = self.parse_expression()?;
        self.expect(TokenKind::Dot, "expected '.' at end of show")?;
        Ok(Stmt::Show(expr))
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        self.advance(); // if
        let cond = self.parse_expression()?;
        if self.check(&TokenKind::Comma) { self.advance(); }
        self.expect(TokenKind::Then, "expected 'then' after if condition")?;
        self.expect(TokenKind::Colon, "expected ':' after then")?;
        let then_branch = self.parse_block()?;
        let mut else_branch = Vec::new();
        self.skip_newlines();
        if self.check(&TokenKind::Otherwise) {
            self.advance();
            self.expect(TokenKind::Colon, "expected ':' after otherwise")?;
            else_branch = self.parse_block()?;
        }
        Ok(Stmt::If { cond, then_branch, else_branch })
    }

    fn parse_while(&mut self) -> Result<Stmt, String> {
        self.advance(); // while
        let cond = self.parse_expression()?;
        self.expect(TokenKind::Repeat, "expected 'repeat' after while condition")?;
        self.expect(TokenKind::Colon, "expected ':' after repeat")?;
        let body = self.parse_block()?;
        Ok(Stmt::While { cond, body })
    }

    fn parse_for(&mut self) -> Result<Stmt, String> {
        self.advance(); // for
        let var = self.expect_ident("expected loop variable")?;
        self.expect(TokenKind::In, "expected 'in' in for")?;
        let iterable = self.parse_expression()?;
        self.expect(TokenKind::Repeat, "expected 'repeat' after for iterable")?;
        self.expect(TokenKind::Colon, "expected ':' after repeat")?;
        let body = self.parse_block()?;
        Ok(Stmt::For { var, iterable, body })
    }

    fn parse_return(&mut self) -> Result<Stmt, String> {
        let span = self.peek(0).span.clone();
        self.advance(); // return
        let value = if self.check(&TokenKind::Dot) { None } else { Some(self.parse_expression()?) };
        self.expect(TokenKind::Dot, "expected '.' at end of return")?;
        Ok(Stmt::Return { value, span })
    }

    fn parse_define(&mut self) -> Result<Stmt, String> {
        let span = self.peek(0).span.clone();
        self.advance(); // define
        let name = self.expect_ident("expected function name")?;
        let params = if self.check(&TokenKind::With) {
            self.advance();
            self.parse_params()?
        } else { Vec::new() };
        let return_type = if self.check(&TokenKind::Returns) {
            self.advance();
            Some(self.parse_type()?)
        } else { None };
        self.expect(TokenKind::Colon, "expected ':' after function signature")?;
        let raw_body = self.parse_block()?;
        let (docstring, body) = self.strip_docstring(raw_body);
        Ok(Stmt::Define { name, params, return_type, docstring, body, span })
    }

    fn strip_docstring(&self, mut stmts: Vec<Stmt>) -> (Option<String>, Vec<Stmt>) {
        let mut docstring = None;
        if let Some(Stmt::Expr(Expr::String(s, _))) = stmts.first() {
            docstring = Some(s.clone());
            stmts.remove(0);
        }
        (docstring, stmts)
    }

    fn parse_init(&mut self) -> Result<Stmt, String> {
        self.advance(); // init
        let params = if self.check(&TokenKind::With) {
            self.advance();
            self.parse_params()?
        } else { Vec::new() };
        self.expect(TokenKind::Colon, "expected ':' after init signature")?;
        let raw_body = self.parse_block()?;
        let (docstring, body) = self.strip_docstring(raw_body);
        Ok(Stmt::Init(Init { params, body, docstring }))
    }

    fn parse_class(&mut self) -> Result<Stmt, String> {
        let span = self.peek(0).span.clone();
        self.advance(); // class
        let name = self.expect_ident("expected class name")?;
        self.expect(TokenKind::Colon, "expected ':' after class name")?;
        let raw_members = self.parse_block()?;
        let (docstring, members) = self.strip_docstring(raw_members);
        let mut init = None;
        let mut methods = Vec::new();
        for m in members {
            match m {
                Stmt::Init(i) => {
                    init = Some(i);
                }
                Stmt::Define { .. } => methods.push(m),
                _ => return Err(self.error_with("class body may only contain init and methods", &m)),
            }
        }
        Ok(Stmt::Class { name, init, methods, docstring, span })
    }

    fn parse_import(&mut self) -> Result<Stmt, String> {
        self.advance(); // import
        let mut paths = Vec::new();
        loop {
            paths.push(self.parse_module_path()?);
            if self.check(&TokenKind::Comma) { self.advance(); continue; }
            if self.check(&TokenKind::And) { self.advance(); continue; }
            break;
        }
        self.expect(TokenKind::Dot, "expected '.' at end of import")?;
        Ok(Stmt::Import(paths))
    }

    fn parse_read(&mut self) -> Result<Stmt, String> {
        self.advance(); // read
        let name = self.expect_ident("expected variable name after read")?;
        self.expect(TokenKind::From, "expected 'from' after variable name in read")?;
        let path = self.parse_expression()?;
        self.expect(TokenKind::Dot, "expected '.' at end of read")?;
        Ok(Stmt::Read { name, path })
    }

    fn parse_write(&mut self) -> Result<Stmt, String> {
        self.advance(); // write
        let content = self.parse_expression()?;
        self.expect(TokenKind::To, "expected 'to' after content in write")?;
        let path = self.parse_expression()?;
        self.expect(TokenKind::Dot, "expected '.' at end of write")?;
        Ok(Stmt::Write { content, path })
    }

    fn parse_try(&mut self) -> Result<Stmt, String> {
        self.advance(); // try
        self.expect(TokenKind::Colon, "expected ':' after try")?;
        let body = self.parse_block()?;
        self.skip_newlines();
        self.expect(TokenKind::Catch, "expected 'catch' after try block")?;
        let catch_var = self.expect_ident("expected variable name after catch")?;
        self.expect(TokenKind::Colon, "expected ':' after catch variable")?;
        let catch_body = self.parse_block()?;
        Ok(Stmt::Try { body, catch_var, catch_body })
    }

    fn parse_export(&mut self) -> Result<Stmt, String> {
        self.advance(); // export
        let mut names = Vec::new();
        loop {
            names.push(self.expect_ident("expected name to export")?);
            if !self.check(&TokenKind::Comma) && !self.check(&TokenKind::And) { break; }
            self.advance();
        }
        self.expect(TokenKind::Dot, "expected '.' at end of export")?;
        Ok(Stmt::Export(names))
    }

    fn parse_module_path(&mut self) -> Result<(String, Span), String> {
        let start = self.peek(0).span.clone();
        let path = self.expect_ident("expected module name")?;
        Ok((path, start))
    }

    fn parse_expr_statement(&mut self) -> Result<Stmt, String> {
        let expr = self.parse_expression()?;
        self.expect(TokenKind::Dot, "expected '.' at end of expression statement")?;
        Ok(Stmt::Expr(expr))
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.skip_newlines();
        self.expect(TokenKind::Indent, "expected indented block")?;
        let mut stmts = Vec::new();
        // A leading string literal that is not followed by '.' is treated as a
        // docstring, allowing stub/interface files to write:
        //   define f with x:
        //       "doc"
        //       ...
        self.skip_newlines();
        if !self.check(&TokenKind::Dedent) && !self.check(&TokenKind::Eof)
            && let TokenKind::String(s) = &self.peek(0).kind
                && !matches!(self.peek(1).kind, TokenKind::Dot) {
                    let span = self.peek(0).span.clone();
                    let s = s.clone();
                    self.advance();
                    stmts.push(Stmt::Expr(Expr::String(s, span)));
                }
        loop {
            self.skip_newlines();
            if self.check(&TokenKind::Dedent) || self.check(&TokenKind::Eof) { break; }
            stmts.push(self.parse_statement()?);
        }
        if self.check(&TokenKind::Dedent) { self.advance(); }
        Ok(stmts)
    }

    fn parse_params(&mut self) -> Result<Vec<(String, Option<String>)>, String> {
        let mut params = Vec::new();
        loop {
            let (type_ann, name) = self.parse_typed_name()?;
            params.push((name, type_ann));
            if !self.check(&TokenKind::Comma) { break; }
            self.advance();
        }
        Ok(params)
    }

    fn is_type_start(&self, name: &str) -> bool {
        matches!(
            name,
            "nothing" | "boolean" | "integer" | "number" | "string" | "list" | "dictionary"
                | "function" | "class"
        ) || name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
    }

    fn parse_type_from(&mut self, name: String) -> Result<String, String> {
        match name.as_str() {
            "list" => {
                self.expect(TokenKind::Of, "expected 'of' after list")?;
                let elem = self.parse_type()?;
                Ok(format!("list of {}", elem))
            }
            "dictionary" => {
                self.expect(TokenKind::Of, "expected 'of' after dictionary")?;
                let key = self.parse_type()?;
                self.expect(TokenKind::To, "expected 'to' after dictionary key type")?;
                let value = self.parse_type()?;
                Ok(format!("dictionary of {} to {}", key, value))
            }
            _ => Ok(name),
        }
    }

    fn parse_type(&mut self) -> Result<String, String> {
        let name = self.expect_ident("expected type name")?;
        self.parse_type_from(name)
    }

    fn parse_typed_name(&mut self) -> Result<(Option<String>, String), String> {
        let first = self.expect_ident("expected name")?;
        if self.is_type_start(&first) {
            // Disambiguate: `let X be 10` (X is the variable) vs `let Person p be ...`
            // (Person is the type). If the next token is an identifier, `first` is a
            // type; if it is `of` after `list`/`dictionary`, parse a compound type.
            // Otherwise `first` is the name itself.
            let is_compound = matches!(first.as_str(), "list" | "dictionary") && self.check(&TokenKind::Of);
            let is_simple_type = matches!(self.peek(0).kind, TokenKind::Ident(_));
            if is_compound || is_simple_type {
                let type_ann = self.parse_type_from(first)?;
                let name = self.expect_ident("expected variable name after type")?;
                Ok((Some(type_ann), name))
            } else {
                Ok((None, first))
            }
        } else {
            Ok((None, first))
        }
    }

    fn expect_ident(&mut self, msg: &str) -> Result<String, String> {
        if let TokenKind::Ident(name) = self.peek(0).kind.clone() {
            self.advance();
            Ok(name)
        } else { Err(self.error(msg)) }
    }

    fn error_with(&self, msg: &str, stmt: &Stmt) -> String {
        format!("parse error: {}: {:?}", msg, stmt)
    }

    // Expressions ---------------------------------------------------------
    pub fn parse_expression(&mut self) -> Result<Expr, String> { self.parse_or() }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.check(&TokenKind::Or) {
            let span = self.peek(0).span.clone();
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Binary { op: BinOp::Or, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_equality()?;
        while self.check(&TokenKind::And) {
            let span = self.peek(0).span.clone();
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::Binary { op: BinOp::And, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;
        loop {
            let (op, span) = if self.check(&TokenKind::EqEq) { (BinOp::Eq, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::NotEq) { (BinOp::Ne, self.peek(0).span.clone()) }
            else { break; };
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        loop {
            let (op, span) = if self.check(&TokenKind::Less) { (BinOp::Lt, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::Greater) { (BinOp::Gt, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::LessEq) { (BinOp::Le, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::GreaterEq) { (BinOp::Ge, self.peek(0).span.clone()) }
            else { break; };
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let (op, span) = if self.check(&TokenKind::Plus) { (BinOp::Add, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::Minus) { (BinOp::Sub, self.peek(0).span.clone()) }
            else { break; };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_power()?;
        loop {
            let (op, span) = if self.check(&TokenKind::Star) { (BinOp::Mul, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::Slash) { (BinOp::Div, self.peek(0).span.clone()) }
            else if self.check(&TokenKind::Percent) { (BinOp::Mod, self.peek(0).span.clone()) }
            else { break; };
            self.advance();
            let right = self.parse_power()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_power(&mut self) -> Result<Expr, String> {
        let left = self.parse_unary()?;
        if self.check(&TokenKind::Power) {
            let span = self.peek(0).span.clone();
            self.advance();
            let right = self.parse_power()?;
            Ok(Expr::Binary { op: BinOp::Pow, left: Box::new(left), right: Box::new(right), span })
        } else { Ok(left) }
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.check(&TokenKind::Minus) {
            let span = self.peek(0).span.clone();
            self.advance();
            Ok(Expr::Unary { op: UnaryOp::Neg, operand: Box::new(self.parse_unary()?), span })
        } else if self.check(&TokenKind::Not) {
            let span = self.peek(0).span.clone();
            self.advance();
            Ok(Expr::Unary { op: UnaryOp::Not, operand: Box::new(self.parse_unary()?), span })
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.check(&TokenKind::With) {
                let span = expr.span().cloned().unwrap_or_else(|| self.peek(0).span.clone());
                self.advance();
                let mut args = Vec::new();
                if !self.is_call_terminator() {
                    // Arguments are full expressions separated by commas, so
                    // `f with a + b, c * d` is `f(a + b, c * d)`.
                    args.push(self.parse_expression()?);
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        args.push(self.parse_expression()?);
                    }
                }
                expr = Expr::Call { callee: Box::new(expr), args, span };
            } else if self.check(&TokenKind::LBracket) {
                let span = expr.span().cloned().unwrap_or_else(|| self.peek(0).span.clone());
                self.advance();
                let index = self.parse_expression()?;
                self.expect(TokenKind::RBracket, "expected ']'")?;
                expr = Expr::Index { object: Box::new(expr), index: Box::new(index), span };
            } else if self.check(&TokenKind::From) {
                // qualified reference: name from module
                let span = expr.span().cloned().unwrap_or_else(|| self.peek(0).span.clone());
                self.advance();
                let module = self.parse_module_path()?.0;
                if let Expr::Variable { name, .. } = expr {
                    expr = Expr::Qualified { name, module, span };
                } else {
                    return Err(self.error("qualified reference must start with a name"));
                }
            } else if self.check(&TokenKind::Dot) && matches!(self.peek(1).kind, TokenKind::Ident(_)) {
                // Property access: obj.prop (compact equivalent of "the prop of obj").
                let span = expr.span().cloned().unwrap_or_else(|| self.peek(0).span.clone());
                self.advance(); // .
                let name = self.expect_ident("expected property name")?;
                // If followed by '(', this is a method call: obj.method(args).
                if self.check(&TokenKind::LParen) {
                    let args = self.parse_parenthesized_arguments()?;
                    expr = Expr::Tell { object: Box::new(expr), method: name, args, span };
                } else {
                    expr = Expr::Property { object: Box::new(expr), name, span };
                }
            } else if self.check(&TokenKind::LParen) {
                // Parenthesized call: f(a, b) (compact equivalent of "f with a, b").
                let span = expr.span().cloned().unwrap_or_else(|| self.peek(0).span.clone());
                let args = self.parse_parenthesized_arguments()?;
                expr = Expr::Call { callee: Box::new(expr), args, span };
            } else if matches!(self.peek(0).kind, TokenKind::Ident(_)) {
                return Err(self.error("property access requires a dot: use 'the <property> of <object>' or '<object>.<property>'"));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn is_call_terminator(&self) -> bool {
        self.check(&TokenKind::Dot) || self.check(&TokenKind::Newline) || self.check(&TokenKind::Indent) || self.check(&TokenKind::Dedent) || self.check(&TokenKind::Eof)
    }

    fn parse_parenthesized_arguments(&mut self) -> Result<Vec<Expr>, String> {
        self.expect(TokenKind::LParen, "expected '('")?;
        let mut args = Vec::new();
        if !self.check(&TokenKind::RParen) {
            args.push(self.parse_expression()?);
            while self.check(&TokenKind::Comma) {
                self.advance();
                args.push(self.parse_expression()?);
            }
        }
        self.expect(TokenKind::RParen, "expected ')'")?;
        Ok(args)
    }

    fn parse_interpolated(&self, parts: Vec<StringPart>) -> Result<Expr, String> {
        let mut exprs: Vec<Expr> = Vec::new();
        for part in parts {
            match part {
                StringPart::Literal(s) => exprs.push(Expr::String(s, Span { line: 1, col: 1 })),
                StringPart::Expr(src) => {
                    let tokens = crate::lexer::Lexer::lex_string(&src)?;
                    let inner = Parser::new(tokens).parse_expression()?;
                    // Wrap the interpolated expression with the built-in `string` function so
                    // numbers, booleans, lists, etc. are automatically converted to text.
                    let span = Span { line: 1, col: 1 };
                    let string_call = Expr::Call {
                        callee: Box::new(Expr::Variable { name: "string".to_string(), span: span.clone() }),
                        args: vec![inner],
                        span: span.clone(),
                    };
                    exprs.push(string_call);
                }
            }
        }
        if exprs.is_empty() {
            return Ok(Expr::String(String::new(), Span { line: 1, col: 1 }));
        }
        let mut result = exprs.remove(0);
        for e in exprs {
            let span = Span { line: 1, col: 1 };
            result = Expr::Binary { op: BinOp::Add, left: Box::new(result), right: Box::new(e), span };
        }
        Ok(result)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek(0).kind.clone() {
            TokenKind::Integer(n) => {
                let span = self.peek(0).span.clone();
                self.advance();
                Ok(Expr::Integer(n, span))
            }
            TokenKind::Number(n) => {
                let span = self.peek(0).span.clone();
                self.advance();
                Ok(Expr::Number(n, span))
            }
            TokenKind::String(s) => {
                let span = self.peek(0).span.clone();
                self.advance();
                Ok(Expr::String(s, span))
            }
            TokenKind::Interpolated(parts) => { self.advance(); self.parse_interpolated(parts) }
            TokenKind::Bool(b) => {
                let span = self.peek(0).span.clone();
                self.advance();
                Ok(Expr::Bool(b, span))
            }
            TokenKind::Nothing => {
                let span = self.peek(0).span.clone();
                self.advance();
                Ok(Expr::Nothing(span))
            }
            TokenKind::Ellipsis => { self.advance(); Ok(Expr::Ellipsis) }
            TokenKind::Ident(name) => {
                let span = self.peek(0).span.clone();
                self.advance();
                Ok(Expr::Variable { name, span })
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                let span = self.peek(0).span.clone();
                self.advance();
                let mut elems = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    elems.push(self.parse_expression()?);
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        elems.push(self.parse_expression()?);
                    }
                }
                self.expect(TokenKind::RBracket, "expected ']'")?;
                Ok(Expr::List(elems, span))
            }
            TokenKind::LBrace => {
                let span = self.peek(0).span.clone();
                self.advance();
                let mut pairs = Vec::new();
                if !self.check(&TokenKind::RBrace) {
                    let key = self.parse_expression()?;
                    self.expect(TokenKind::Colon, "expected ':' after dict key")?;
                    let value = self.parse_expression()?;
                    pairs.push((key, value));
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        let key = self.parse_expression()?;
                        self.expect(TokenKind::Colon, "expected ':' after dict key")?;
                        let value = self.parse_expression()?;
                        pairs.push((key, value));
                    }
                }
                self.expect(TokenKind::RBrace, "expected '}'")?;
                Ok(Expr::Dict(pairs, span))
            }
            TokenKind::New => {
                let span = self.peek(0).span.clone();
                self.advance();
                let class = self.parse_primary()?;
                let mut args = Vec::new();
                if self.check(&TokenKind::With) {
                    self.advance();
                    if !self.is_call_terminator() {
                        args.push(self.parse_expression()?);
                        while self.check(&TokenKind::Comma) {
                            self.advance();
                            args.push(self.parse_expression()?);
                        }
                    }
                } else if self.check(&TokenKind::LParen) {
                    args = self.parse_parenthesized_arguments()?;
                }
                Ok(Expr::New { class: Box::new(class), args, span })
            }
            TokenKind::Tell => {
                let span = self.peek(0).span.clone();
                self.advance();
                let object = self.parse_expression()?;
                self.expect(TokenKind::To, "expected 'to' after tell object")?;
                let method = self.expect_ident("expected method name")?;
                if self.check(&TokenKind::LParen) {
                    let msg = format!("method calls use parentheses directly on the object: try `obj.{}(...)` or use `tell ... to ... with ...`", method);
                    return Err(self.error(&msg));
                }
                let mut args = Vec::new();
                if self.check(&TokenKind::With) {
                    self.advance();
                    if !self.is_call_terminator() {
                        args.push(self.parse_expression()?);
                        while self.check(&TokenKind::Comma) {
                            self.advance();
                            args.push(self.parse_expression()?);
                        }
                    }
                }
                Ok(Expr::Tell { object: Box::new(object), method, args, span })
            }
            TokenKind::The => {
                let span = self.peek(0).span.clone();
                self.advance();
                let name = self.expect_ident("expected property name")?;
                self.expect(TokenKind::Of, "expected 'of' after property name")?;
                // Use parse_primary so `the name of this + "!"` means `(the name of this) + "!"`.
                let object = self.parse_primary()?;
                Ok(Expr::Property { object: Box::new(object), name, span })
            }
            _ => Err(self.error("expected expression")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(source: &str) -> Program {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let t = lexer.next_token().unwrap();
            let eof = matches!(t.kind, TokenKind::Eof);
            tokens.push(t);
            if eof { break; }
        }
        Parser::new(tokens).parse_program().unwrap()
    }

    #[test]
    fn parse_let_statement() {
        let prog = parse("let x be 10.");
        assert_eq!(prog.statements.len(), 1);
        assert!(matches!(prog.statements[0], Stmt::Let { .. }));
    }

    #[test]
    fn parse_function_definition() {
        let prog = parse("define add with number a, number b returns number:\n    return a + b.");
        assert_eq!(prog.statements.len(), 1);
        assert!(matches!(prog.statements[0], Stmt::Define { .. }));
    }

    #[test]
    fn parse_list_literal_with_span() {
        let prog = parse("let xs be [1, 2, 3].");
        if let Stmt::Let { value: Expr::List(_, span), .. } = &prog.statements[0] {
            assert_eq!(span.line, 1);
            assert_eq!(span.col, 11); // opening bracket '['
        } else {
            panic!("expected let with list literal");
        }
    }

    #[test]
    fn parse_string_literal_span_in_index() {
        let prog = parse("let xs be [1, 2, 3].\nshow xs[\"a\"].");
        if let Stmt::Show(Expr::Index { index, .. }) = &prog.statements[1] {
            if let Expr::String(_, span) = index.as_ref() {
                assert_eq!(span.line, 2);
                assert_eq!(span.col, 9); // opening quote of "a"
                return;
            }
        }
        panic!("expected index with string literal");
    }

    #[test]
    fn parse_error_missing_be() {
        let mut lexer = Lexer::new("let x.");
        let mut tokens = Vec::new();
        loop {
            let t = lexer.next_token().unwrap();
            let eof = matches!(t.kind, TokenKind::Eof);
            tokens.push(t);
            if eof { break; }
        }
        let result = Parser::new(tokens).parse_program();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected 'be'"));
    }

    fn expr_to_string(expr: &Expr) -> String {
        match expr {
            Expr::Call { callee, args, .. } => {
                let callee = expr_to_string(callee);
                let args = args.iter().map(expr_to_string).collect::<Vec<_>>().join(", ");
                format!("{}({})", callee, args)
            }
            Expr::Binary { op, left, right, .. } => {
                format!("({} {} {})", expr_to_string(left), op, expr_to_string(right))
            }
            Expr::Variable { name, .. } => name.clone(),
            Expr::Integer(n, _) => n.to_string(),
            Expr::Bool(b, _) => b.to_string(),
            _ => format!("{:?}", expr),
        }
    }

    #[test]
    fn call_argument_is_a_full_expression() {
        let prog = parse("show f with 3 + 4.");
        if let Stmt::Show(Expr::Call { args, .. }) = &prog.statements[0] {
            assert_eq!(args.len(), 1);
            assert_eq!(expr_to_string(&args[0]), "(3 + 4)");
        } else {
            panic!("expected call expression");
        }
    }

    #[test]
    fn call_argument_can_contain_boolean_operator() {
        let prog = parse("show f with true and false.");
        if let Stmt::Show(Expr::Call { args, .. }) = &prog.statements[0] {
            assert_eq!(args.len(), 1);
            assert_eq!(expr_to_string(&args[0]), "(true and false)");
        } else {
            panic!("expected call expression");
        }
    }

    #[test]
    fn call_argument_can_contain_comparison() {
        let prog = parse("show f with 3 > 2.");
        if let Stmt::Show(Expr::Call { args, .. }) = &prog.statements[0] {
            assert_eq!(args.len(), 1);
            assert_eq!(expr_to_string(&args[0]), "(3 > 2)");
        } else {
            panic!("expected call expression");
        }
    }

    #[test]
    fn multi_argument_call_uses_full_expressions() {
        let prog = parse("show f with 1 + 2, 3 * 4.");
        if let Stmt::Show(Expr::Call { args, .. }) = &prog.statements[0] {
            assert_eq!(args.len(), 2);
            assert_eq!(expr_to_string(&args[0]), "(1 + 2)");
            assert_eq!(expr_to_string(&args[1]), "(3 * 4)");
        } else {
            panic!("expected call expression");
        }
    }

    #[test]
    fn parentheses_can_group_call_argument() {
        let prog = parse("show f with (3 + 4).");
        if let Stmt::Show(Expr::Call { args, .. }) = &prog.statements[0] {
            assert_eq!(args.len(), 1);
            assert_eq!(expr_to_string(&args[0]), "(3 + 4)");
        } else {
            panic!("expected call expression");
        }
    }

    #[test]
    fn dot_property_access_is_parsed() {
        let prog = parse("show obj.prop.");
        if let Stmt::Show(Expr::Property { object, name, .. }) = &prog.statements[0] {
            assert_eq!(expr_to_string(object), "obj");
            assert_eq!(name, "prop");
        } else {
            panic!("expected property expression, got {:?}", prog.statements[0]);
        }
    }

    #[test]
    fn dot_property_assignment_is_parsed() {
        let prog = parse("set obj.prop to 42.");
        if let Stmt::Set { target: AssignTarget::Property { object, name, .. }, value } = &prog.statements[0] {
            assert_eq!(expr_to_string(object), "obj");
            assert_eq!(name, "prop");
            assert_eq!(expr_to_string(value), "42");
        } else {
            panic!("expected property assignment, got {:?}", prog.statements[0]);
        }
    }

    #[test]
    fn parenthesized_call_is_parsed() {
        let prog = parse("show add(1, 2).");
        if let Stmt::Show(Expr::Call { callee, args, .. }) = &prog.statements[0] {
            assert_eq!(expr_to_string(callee), "add");
            assert_eq!(args.len(), 2);
            assert_eq!(expr_to_string(&args[0]), "1");
            assert_eq!(expr_to_string(&args[1]), "2");
        } else {
            panic!("expected parenthesized call, got {:?}", prog.statements[0]);
        }
    }

    #[test]
    fn parenthesized_method_call_is_parsed() {
        let prog = parse("show obj.method(1, 2).");
        if let Stmt::Show(Expr::Tell { object, method, args, .. }) = &prog.statements[0] {
            assert_eq!(expr_to_string(object), "obj");
            assert_eq!(method, "method");
            assert_eq!(args.len(), 2);
        } else {
            panic!("expected method call, got {:?}", prog.statements[0]);
        }
    }

    #[test]
    fn parenthesized_constructor_is_parsed() {
        let prog = parse("let p be new Person(\"Ada\", 42).");
        if let Stmt::Let { value: Expr::New { class, args, .. }, .. } = &prog.statements[0] {
            assert_eq!(expr_to_string(class), "Person");
            assert_eq!(args.len(), 2);
        } else {
            panic!("expected constructor, got {:?}", prog.statements[0]);
        }
    }
}

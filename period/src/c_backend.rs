use std::collections::HashSet;
use crate::ast::*;

pub fn try_compile_c(program: &Program, source_path: &str) -> Option<(String, Vec<(usize, Span)>)> {
    let mut generator = CGen::new(source_path);
    if generator.gen_program(program).is_ok() {
        Some((generator.output, generator.line_map))
    } else {
        None
    }
}

#[derive(Debug)]
struct Unsupported;

struct CGen {
    output: String,
    indent: usize,
    locals: Vec<HashSet<String>>,
    globals: HashSet<String>,
    source_path: String,
    stmt_span: Option<Span>,
    line_map: Vec<(usize, Span)>,
}

impl CGen {
    fn new(source_path: &str) -> Self {
        Self { output: String::new(), indent: 0, locals: Vec::new(), globals: HashSet::new(), source_path: source_path.to_string(), stmt_span: None, line_map: Vec::new() }
    }

    fn escape_c_string(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r")
    }

    fn expr_span(expr: &Expr) -> Option<&Span> {
        match expr {
            Expr::Variable { span, .. } | Expr::Binary { span, .. } => Some(span),
            _ => None,
        }
    }

    fn stmt_span(stmt: &Stmt) -> Option<&Span> {
        match stmt {
            Stmt::Show(e) | Stmt::Let { value: e, .. } | Stmt::Set { value: e, .. } | Stmt::Expr(e) | Stmt::Return(Some(e)) => Self::expr_span(e),
            Stmt::If { cond, .. } | Stmt::While { cond, .. } => Self::expr_span(cond),
            Stmt::For { iterable, .. } => Self::expr_span(iterable),
            _ => None,
        }
    }

    fn unsupported<T>() -> Result<T, Unsupported> { Err(Unsupported) }

    fn in_function(&self) -> bool { !self.locals.is_empty() }

    fn current_locals(&self) -> &HashSet<String> {
        self.locals.last().unwrap_or(&self.globals)
    }

    fn is_local(&self, name: &str) -> bool {
        self.locals.iter().any(|scope| scope.contains(name))
    }

    fn line(&mut self, s: &str) {
        for _ in 0..self.indent { self.output.push_str("    "); }
        self.output.push_str(s);
        self.output.push('\n');
        if let Some(span) = &self.stmt_span {
            let c_line = self.output.matches('\n').count();
            self.line_map.push((c_line, span.clone()));
        }
    }

    fn gen_program(&mut self, program: &Program) -> Result<(), Unsupported> {
        self.line("#include <stdio.h>");
        self.line("#include <stdlib.h>");
        self.line("#include <string.h>");
        self.line("");
        self.line(&format!("static const char *period_source_path = \"{}\";", Self::escape_c_string(&self.source_path)));
        self.line("");
        self.line("static void period_runtime_error(int line, int col, const char *msg) {");
        self.indent += 1;
        self.line("fprintf(stderr, \"%s:%d:%d: runtime error: %s\\n\", period_source_path, line, col, msg);");
        self.line("FILE *f = fopen(period_source_path, \"r\");");
        self.line("if (f) {");
        self.indent += 1;
        self.line("char buf[1024];");
        self.line("int cur = 1;");
        self.line("while (fgets(buf, sizeof(buf), f)) {");
        self.indent += 1;
        self.line("size_t len = strlen(buf);");
        self.line("while (len > 0 && (buf[len-1] == '\\n' || buf[len-1] == '\\r')) { buf[--len] = '\\0'; }");
        self.line("if (cur == line) {");
        self.indent += 1;
        self.line("fprintf(stderr, \"    %d | %s\\n\", line, buf);");
        self.line("fprintf(stderr, \"%*s^\\n\", 7 + col - 1, \"\");");
        self.line("break;");
        self.indent -= 1;
        self.line("}");
        self.line("cur++;");
        self.indent -= 1;
        self.line("}");
        self.line("fclose(f);");
        self.indent -= 1;
        self.line("}");
        self.line("exit(1);");
        self.indent -= 1;
        self.line("}");
        self.line("");
        self.line("static long long period_pow(long long base, long long exp) {");
        self.indent += 1;
        self.line("long long result = 1;");
        self.line("while (exp > 0) { result *= base; exp--; }");
        self.line("return result;");
        self.indent -= 1;
        self.line("}");
        self.line("");
        self.line("static long long period_div(long long a, long long b, int line, int col) {");
        self.indent += 1;
        self.line("if (b == 0) { period_runtime_error(line, col, \"Division by zero.\"); }");
        self.line("return a / b;");
        self.indent -= 1;
        self.line("}");
        self.line("");
        self.line("static long long period_mod(long long a, long long b, int line, int col) {");
        self.indent += 1;
        self.line("if (b == 0) { period_runtime_error(line, col, \"Modulo by zero.\"); }");
        self.line("return a % b;");
        self.indent -= 1;
        self.line("}");
        self.line("");

        // Emit top-level functions first.
        for stmt in &program.statements {
            if let Stmt::Define { .. } = stmt {
                self.gen_toplevel_stmt(stmt)?;
            }
        }

        // Wrap top-level statements in an exported function that the host loads.
        self.line("__declspec(dllexport) int period_run(void) {");
        self.indent += 1;
        for stmt in &program.statements {
            if !matches!(stmt, Stmt::Define { .. }) {
                self.gen_stmt(stmt)?;
            }
        }
        self.line("return 0;");
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    fn gen_toplevel_stmt(&mut self, stmt: &Stmt) -> Result<(), Unsupported> {
        match stmt {
            Stmt::Define { name, params, body, .. } => self.gen_define(name, params, body),
            _ => self.gen_stmt(stmt),
        }
    }

    fn gen_define(&mut self, name: &str, params: &[(String, Option<String>)], body: &[Stmt]) -> Result<(), Unsupported> {
        let params_str = params.iter().map(|(n, _)| format!("long long {}", n)).collect::<Vec<_>>().join(", ");
        self.line(&format!("static long long {}({}) {{", name, params_str));
        self.indent += 1;
        let mut locals: HashSet<String> = params.iter().map(|(n, _)| n.clone()).collect();
        for stmt in body {
            Self::collect_locals(stmt, &mut locals);
        }
        self.locals.push(locals);
        for stmt in body { self.gen_stmt(stmt)?; }
        self.line("return 0;");
        self.indent -= 1;
        self.line("}");
        self.locals.pop();
        Ok(())
    }

    fn collect_locals(stmt: &Stmt, locals: &mut HashSet<String>) {
        match stmt {
            Stmt::Let { name, .. } => { locals.insert(name.clone()); }
            Stmt::If { then_branch, else_branch, .. } => {
                for s in then_branch { Self::collect_locals(s, locals); }
                for s in else_branch { Self::collect_locals(s, locals); }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                for s in body { Self::collect_locals(s, locals); }
            }
            _ => {}
        }
    }

    fn gen_stmt(&mut self, stmt: &Stmt) -> Result<(), Unsupported> {
        let prev_span = self.stmt_span.take();
        self.stmt_span = Self::stmt_span(stmt).cloned();
        match stmt {
            Stmt::Let { name, value } => {
                self.globals.insert(name.clone());
                let val = self.gen_expr(value)?;
                self.line(&format!("long long {} = {};", name, val));
            }
            Stmt::Set { target, value } => {
                let target_str = self.gen_assign_target(target)?;
                let val = self.gen_expr(value)?;
                self.line(&format!("{} = {};", target_str, val));
            }
            Stmt::Show(expr) => {
                let val = self.gen_expr(expr)?;
                self.line(&format!("printf(\"%lld\\n\", {});", val));
            }
            Stmt::If { cond, then_branch, else_branch } => {
                let c = self.gen_cond(cond)?;
                self.line(&format!("if ({}) {{", c));
                self.indent += 1;
                for s in then_branch { self.gen_stmt(s)?; }
                self.indent -= 1;
                if else_branch.is_empty() {
                    self.line("}");
                } else {
                    self.line("} else {");
                    self.indent += 1;
                    for s in else_branch { self.gen_stmt(s)?; }
                    self.indent -= 1;
                    self.line("}");
                }
            }
            Stmt::While { cond, body } => {
                let c = self.gen_cond(cond)?;
                self.line(&format!("while ({}) {{", c));
                self.indent += 1;
                for s in body { self.gen_stmt(s)?; }
                self.indent -= 1;
                self.line("}");
            }
            Stmt::For { var, iterable, body } => {
                let iter = self.gen_iterable(iterable)?;
                self.line(&format!("for (long long {var} = 0; {var} < {iter}; {var}++) {{"));
                self.indent += 1;
                for s in body { self.gen_stmt(s)?; }
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Return(Some(expr)) => {
                let val = self.gen_expr(expr)?;
                self.line(&format!("return {};", val));
            }
            Stmt::Return(None) => self.line("return 0;"),
            Stmt::Expr(expr) => { self.gen_expr(expr)?; }
            _ => return Self::unsupported(),
        }
        self.stmt_span = prev_span;
        Ok(())
    }

    fn gen_assign_target(&mut self, target: &AssignTarget) -> Result<String, Unsupported> {
        match target {
            AssignTarget::Variable { name, .. } => Ok(name.clone()),
            AssignTarget::Index { .. } => Self::unsupported(),
            AssignTarget::Property { .. } => Self::unsupported(),
        }
    }

    fn gen_iterable(&mut self, expr: &Expr) -> Result<String, Unsupported> {
        match expr {
            Expr::Call { callee, args } => {
                if let Expr::Variable { name, .. } = callee.as_ref() {
                    if name == "range" {
                        let args_str: Result<Vec<_>, _> = args.iter().map(|a| self.gen_expr(a)).collect();
                        return Ok(args_str?.join(""));
                    }
                }
                Self::unsupported()
            }
            _ => Self::unsupported(),
        }
    }

    fn gen_cond(&mut self, expr: &Expr) -> Result<String, Unsupported> {
        // Conditions in C can use raw comparisons (non-zero == true).
        match expr {
            Expr::Bool(b) => Ok(if *b { "1" } else { "0" }.to_string()),
            Expr::Number(n) => Ok(format!("{}LL", *n as i64)),
            Expr::Variable { name, .. } => Ok(name.clone()),
            Expr::Binary { op, left, right, .. } => {
                let l = self.gen_expr(left)?;
                let r = self.gen_expr(right)?;
                match op {
                    BinOp::Eq => Ok(format!("({}) == ({})", l, r)),
                    BinOp::Ne => Ok(format!("({}) != ({})", l, r)),
                    BinOp::Lt => Ok(format!("({}) < ({})", l, r)),
                    BinOp::Gt => Ok(format!("({}) > ({})", l, r)),
                    BinOp::Le => Ok(format!("({}) <= ({})", l, r)),
                    BinOp::Ge => Ok(format!("({}) >= ({})", l, r)),
                    BinOp::And => Ok(format!("({}) && ({})", l, r)),
                    BinOp::Or => Ok(format!("({}) || ({})", l, r)),
                    _ => Ok(format!("({}) != 0", self.gen_expr(expr)?)),
                }
            }
            Expr::Unary { op, operand } => {
                let v = self.gen_expr(operand)?;
                match op {
                    UnaryOp::Neg => Ok(format!("(-{}) != 0", v)),
                    UnaryOp::Not => Ok(format!("!{}", v)),
                }
            }
            _ => Ok(format!("({}) != 0", self.gen_expr(expr)?)),
        }
    }

    fn gen_expr(&mut self, expr: &Expr) -> Result<String, Unsupported> {
        match expr {
            Expr::Number(n) => Ok(format!("{}LL", *n as i64)),
            Expr::Bool(b) => Ok(if *b { "1LL" } else { "0LL" }.to_string()),
            Expr::Variable { name, .. } => Ok(name.clone()),
            Expr::Binary { op, left, right, span } => {
                let l = self.gen_expr(left)?;
                let r = self.gen_expr(right)?;
                match op {
                    BinOp::Add => Ok(format!("({} + {})", l, r)),
                    BinOp::Sub => Ok(format!("({} - {})", l, r)),
                    BinOp::Mul => Ok(format!("({} * {})", l, r)),
                    BinOp::Div => Ok(format!("period_div({}, {}, {}, {})", l, r, span.line, span.col)),
                    BinOp::Mod => Ok(format!("period_mod({}, {}, {}, {})", l, r, span.line, span.col)),
                    BinOp::Pow => Ok(format!("period_pow({}, {})", l, r)),
                    BinOp::Eq => Ok(format!("(({}) == ({}) ? 1LL : 0LL)", l, r)),
                    BinOp::Ne => Ok(format!("(({}) != ({}) ? 1LL : 0LL)", l, r)),
                    BinOp::Lt => Ok(format!("(({}) < ({}) ? 1LL : 0LL)", l, r)),
                    BinOp::Gt => Ok(format!("(({}) > ({}) ? 1LL : 0LL)", l, r)),
                    BinOp::Le => Ok(format!("(({}) <= ({}) ? 1LL : 0LL)", l, r)),
                    BinOp::Ge => Ok(format!("(({}) >= ({}) ? 1LL : 0LL)", l, r)),
                    BinOp::And => Ok(format!("(({}) && ({}) ? 1LL : 0LL)", l, r)),
                    BinOp::Or => Ok(format!("(({}) || ({}) ? 1LL : 0LL)", l, r)),
                }
            }
            Expr::Unary { op, operand } => {
                let v = self.gen_expr(operand)?;
                match op {
                    UnaryOp::Neg => Ok(format!("(-{})", v)),
                    UnaryOp::Not => Ok(format!("(!{} ? 1LL : 0LL)", v)),
                }
            }
            Expr::Call { callee, args } => {
                let name = match callee.as_ref() {
                    Expr::Variable { name, .. } => name.clone(),
                    _ => return Self::unsupported(),
                };
                let args_str: Result<Vec<_>, _> = args.iter().map(|a| self.gen_expr(a)).collect();
                Ok(format!("{}({})", name, args_str?.join(", ")))
            }
            _ => Self::unsupported(),
        }
    }
}

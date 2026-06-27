use std::collections::HashSet;
use crate::ast::*;

pub fn try_compile(program: &Program) -> Option<String> {
    let mut generator = RustGen::new();
    if generator.gen_program(program).is_ok() {
        Some(generator.output)
    } else {
        None
    }
}

#[derive(Debug)]
struct Unsupported;

struct RustGen {
    output: String,
    indent: usize,
    locals: Vec<HashSet<String>>,
    globals: HashSet<String>,
}

impl RustGen {
    fn new() -> Self {
        Self { output: String::new(), indent: 0, locals: Vec::new(), globals: HashSet::new() }
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
    }

    fn gen_program(&mut self, program: &Program) -> Result<(), Unsupported> {
        self.line("fn main() {");
        self.indent += 1;
        for stmt in &program.statements {
            self.gen_toplevel_stmt(stmt)?;
        }
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
        let params_str = params.iter().map(|(n, _)| format!("mut {}: i64", n)).collect::<Vec<_>>().join(", ");
        self.line(&format!("fn {}({}) -> i64 {{", name, params_str));
        self.indent += 1;
        let mut locals: HashSet<String> = params.iter().map(|(n, _)| n.clone()).collect();
        for stmt in body {
            Self::collect_locals(stmt, &mut locals);
        }
        self.locals.push(locals);
        for stmt in body { self.gen_stmt(stmt)?; }
        self.line("0");
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
        match stmt {
            Stmt::Let { name, value } => {
                self.globals.insert(name.clone());
                let val = self.gen_expr(value)?;
                self.line(&format!("let mut {} = {};", name, val));
            }
            Stmt::Set { target, value } => {
                let target_str = self.gen_assign_target(target)?;
                let val = self.gen_expr(value)?;
                self.line(&format!("{} = {};", target_str, val));
            }
            Stmt::Show(expr) => {
                let val = self.gen_expr(expr)?;
                self.line(&format!("println!(\"{{}}\", {});", val));
            }
            Stmt::If { cond, then_branch, else_branch } => {
                let c = self.gen_expr(cond)?;
                self.line(&format!("if {} != 0 {{", c));
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
                let c = self.gen_expr(cond)?;
                self.line(&format!("while {} != 0 {{", c));
                self.indent += 1;
                for s in body { self.gen_stmt(s)?; }
                self.indent -= 1;
                self.line("}");
            }
            Stmt::For { var, iterable, body } => {
                let iter = self.gen_iterable(iterable)?;
                self.line(&format!("for mut {} in {} {{", var, iter));
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
        Ok(())
    }

    fn gen_assign_target(&mut self, target: &AssignTarget) -> Result<String, Unsupported> {
        match target {
            AssignTarget::Variable(name) => Ok(name.clone()),
            AssignTarget::Index { .. } => Self::unsupported(),
            AssignTarget::Property { .. } => Self::unsupported(),
        }
    }

    fn gen_iterable(&mut self, expr: &Expr) -> Result<String, Unsupported> {
        match expr {
            Expr::Call { callee, args } => {
                if let Expr::Variable(name) = callee.as_ref() {
                    if name == "range" {
                        let args_str: Result<Vec<_>, _> = args.iter().map(|a| self.gen_expr(a)).collect();
                        return Ok(format!("0i64..{}", args_str?.join("..")));
                    }
                }
                Self::unsupported()
            }
            _ => Self::unsupported(),
        }
    }

    fn gen_expr(&mut self, expr: &Expr) -> Result<String, Unsupported> {
        match expr {
            Expr::Number(n) => Ok(format!("{}i64", *n as i64)),
            Expr::Bool(b) => Ok(if *b { "1i64" } else { "0i64" }.to_string()),
            Expr::Variable(name) => Ok(name.clone()),
            Expr::Binary { op, left, right } => {
                let l = self.gen_expr(left)?;
                let r = self.gen_expr(right)?;
                match op {
                    BinOp::Add => Ok(format!("({} + {})", l, r)),
                    BinOp::Sub => Ok(format!("({} - {})", l, r)),
                    BinOp::Mul => Ok(format!("({} * {})", l, r)),
                    BinOp::Div => Ok(format!("({} / {})", l, r)),
                    BinOp::Mod => Ok(format!("({} % {})", l, r)),
                    BinOp::Pow => Ok(format!("{}.pow({} as u32)", l, r)),
                    BinOp::Eq => Ok(format!("(({} == {}) as i64)", l, r)),
                    BinOp::Ne => Ok(format!("(({} != {}) as i64)", l, r)),
                    BinOp::Lt => Ok(format!("(({} < {}) as i64)", l, r)),
                    BinOp::Gt => Ok(format!("(({} > {}) as i64)", l, r)),
                    BinOp::Le => Ok(format!("(({} <= {}) as i64)", l, r)),
                    BinOp::Ge => Ok(format!("(({} >= {}) as i64)", l, r)),
                    BinOp::And => Ok(format!("((({} != 0) && ({} != 0)) as i64)", l, r)),
                    BinOp::Or => Ok(format!("((({} != 0) || ({} != 0)) as i64)", l, r)),
                }
            }
            Expr::Unary { op, operand } => {
                let o = self.gen_expr(operand)?;
                match op {
                    UnaryOp::Neg => Ok(format!("(-{})", o)),
                    UnaryOp::Not => Ok(format!("((!({} != 0)) as i64)", o)),
                }
            }
            Expr::Call { callee, args } => {
                let callee_str = self.gen_expr(callee)?;
                let args_str: Result<Vec<_>, _> = args.iter().map(|a| self.gen_expr(a)).collect();
                Ok(format!("{}({})", callee_str, args_str?.join(", ")))
            }
            _ => Self::unsupported(),
        }
    }
}

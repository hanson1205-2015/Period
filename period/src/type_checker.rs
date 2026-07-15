//! Static type checker for Period.
//!
//! This is a pre-execution pass: it walks the AST, uses existing type
//! annotations where present, infers simple literal types, and reports type
//! errors with source locations. It does not change the runtime behaviour; the
//! interpreter still enforces types at runtime as a safety net.

use num_bigint::BigInt;

use crate::ast::{AssignTarget, BinOp, Expr, Init, Program, Span, Stmt, UnaryOp};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::types::{parse_type_ann, ClassInfo, Type};
use std::collections::HashMap;

/// A diagnostic message paired with its source location.
pub type Diagnostic = (Span, String);

pub struct TypeChecker {
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
    scopes: Vec<HashMap<String, Type>>,
    return_types: Vec<Type>,
    classes: HashMap<String, ClassInfo>,
    modules: HashMap<String, HashMap<String, Type>>,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut builtins = HashMap::new();
        // Precise signatures where the built-in only accepts certain types;
        // `string`/`boolean`/`type` genuinely accept anything.
        let sized = Type::Union(vec![
            Type::String,
            Type::List(Box::new(Type::Anything)),
            Type::Dict(Box::new(Type::Anything), Box::new(Type::Anything)),
            Type::Range,
        ]);
        builtins.insert("length".to_string(), Type::Function(vec![sized], Box::new(Type::Integer)));
        builtins.insert("string".to_string(), Type::Function(vec![Type::Unknown], Box::new(Type::String)));
        let numeric = Type::Union(vec![Type::Integer, Type::Number, Type::String, Type::Boolean]);
        builtins.insert("number".to_string(), Type::Function(vec![numeric.clone()], Box::new(Type::Number)));
        builtins.insert("integer".to_string(), Type::Function(vec![numeric], Box::new(Type::Integer)));
        builtins.insert("boolean".to_string(), Type::Function(vec![Type::Unknown], Box::new(Type::Boolean)));
        builtins.insert("type".to_string(), Type::Function(vec![Type::Unknown], Box::new(Type::String)));
        // `input` is a zero-arity built-in that is auto-called when used as a value,
        // so its value type is its return type rather than a function type.
        builtins.insert("input".to_string(), Type::String);
        // `range` is variadic (1-3 integer arguments); its call site is checked specially.
        builtins.insert("range".to_string(), Type::Function(vec![Type::Integer], Box::new(Type::Range)));
        // `error` raises a runtime error with a message.
        builtins.insert("error".to_string(), Type::Function(vec![Type::String], Box::new(Type::Nothing)));
        // `append` mutates a list by adding an element to the end.
        builtins.insert("append".to_string(), Type::Function(vec![Type::List(Box::new(Type::Anything)), Type::Anything], Box::new(Type::Nothing)));
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            scopes: vec![builtins],
            return_types: Vec::new(),
            classes: HashMap::new(),
            modules: HashMap::new(),
        }
    }

    pub fn check(&mut self, program: &Program) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
        // First pass: collect class definitions, function signatures, and module imports.
        for stmt in &program.statements {
            self.collect_top_level(stmt);
        }
        // Second pass: type-check each statement.
        for stmt in &program.statements {
            self.check_stmt(stmt);
        }
        (self.errors.clone(), self.warnings.clone())
    }

    fn collect_top_level(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Define { name, params, return_type, body, .. } => {
                let arg_types: Vec<Type> = params
                    .iter()
                    .map(|(_, ann)| ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown))
                    .collect();
                let ret = return_type.as_deref().map(parse_type_ann).unwrap_or_else(|| Self::infer_return_type(body));
                let func_type = Type::Function(arg_types, Box::new(ret));
                self.define(name, func_type);
            }
            Stmt::Class { name, init, methods, .. } => {
                let mut info = ClassInfo::default();
                if let Some(Init { params, body, .. }) = init {
                    let mut fields = HashMap::new();
                    let mut param_types = HashMap::new();
                    for (p_name, ann) in params {
                        let ty = ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown);
                        param_types.insert(p_name.clone(), ty.clone());
                        fields.insert(p_name.clone(), ty);
                    }
                    info.init_params = params.iter()
                        .map(|(_, ann)| ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown))
                        .collect();
                    // Fields assigned via `set the <name> of this to ...` in init body
                    // should also be visible to the type checker.
                    for stmt in body {
                        if let Stmt::Set {
                            target: AssignTarget::Property { object, name: field_name, .. },
                            value,
                        } = stmt
                            && let Expr::Variable { name: obj_name, .. } = object.as_ref()
                                && obj_name == "this" && !fields.contains_key(field_name) {
                                    fields.insert(
                                        field_name.clone(),
                                        Self::infer_init_field_type(value, &param_types),
                                    );
                                }
                    }
                    info.fields = fields;
                }
                for m in methods {
                    if let Stmt::Define { name: mname, params, return_type, body, .. } = m {
                        let arg_types: Vec<Type> = params
                            .iter()
                            .map(|(_, ann)| ann.as_deref().map(parse_type_ann).unwrap_or(Type::Anything))
                            .collect();
                        let ret = return_type.as_deref().map(parse_type_ann).unwrap_or_else(|| Self::infer_return_type(body));
                        info.methods.insert(mname.clone(), Type::Function(arg_types, Box::new(ret)));
                    }
                }
                self.classes.insert(name.clone(), info);
                self.define(name, Type::Class(name.clone()));
            }
            Stmt::Import(paths) => {
                for (path, _) in paths {
                    self.collect_module(path);
                }
            }
            _ => {}
        }
    }

    /// Infer a function's return type from its body when no explicit annotation is given.
    /// Bodies with no returns yield `nothing`; returns whose inferred types all
    /// agree yield that type; conflicting concrete types yield a union. If any
    /// return's type cannot be inferred, the result is `Anything`.
    fn infer_return_type(body: &[Stmt]) -> Type {
        let mut returns: Vec<&Expr> = Vec::new();
        Self::collect_returns(body, &mut returns);
        if returns.is_empty() {
            return Type::Nothing;
        }
        let inferred: Vec<Type> = returns.iter().map(|e| Self::infer_expr_type(e)).collect();
        if inferred.iter().any(|t| matches!(t, Type::Anything | Type::Error)) {
            return Type::Anything;
        }
        let mut members: Vec<Type> = Vec::new();
        for t in inferred {
            if !members.contains(&t) {
                members.push(t);
            }
        }
        match members.len() {
            0 => Type::Anything,
            1 => members.pop().unwrap_or(Type::Anything),
            _ => Type::Union(members),
        }
    }

    fn collect_returns<'a>(stmts: &'a [Stmt], out: &mut Vec<&'a Expr>) {
        for stmt in stmts {
            match stmt {
                Stmt::Return { value: Some(expr), .. } => out.push(expr),
                Stmt::Return { value: None, .. } => {}
                Stmt::If { then_branch, else_branch, .. } => {
                    Self::collect_returns(then_branch, out);
                    Self::collect_returns(else_branch, out);
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    Self::collect_returns(body, out);
                }
                Stmt::Try { body, catch_body, .. } => {
                    Self::collect_returns(body, out);
                    Self::collect_returns(catch_body, out);
                }
                Stmt::Define { body, .. } => Self::collect_returns(body, out),
                Stmt::Class { init, methods, .. } => {
                    if let Some(init) = init {
                        Self::collect_returns(&init.body, out);
                    }
                    for m in methods {
                        if let Stmt::Define { body, .. } = m {
                            Self::collect_returns(body, out);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn infer_expr_type(expr: &Expr) -> Type {
        match expr {
            Expr::Integer(_, _) => Type::Integer,
            Expr::Number(_, _) => Type::Number,
            Expr::String(_, _) => Type::String,
            Expr::Bool(_, _) => Type::Boolean,
            Expr::Nothing(_) => Type::Nothing,
            Expr::Unary { op, operand, .. } => match op {
                UnaryOp::Neg => {
                    let t = Self::infer_expr_type(operand);
                    if t == Type::Integer { Type::Integer } else { Type::Number }
                }
                UnaryOp::Not => Type::Boolean,
            },
            Expr::Binary { op, left, right, .. } => match op {
                BinOp::Add => {
                    let lt = Self::infer_expr_type(left);
                    let rt = Self::infer_expr_type(right);
                    if lt == Type::String || rt == Type::String { Type::String }
                    else if (lt == Type::Integer || lt == Type::Anything || lt == Type::Unknown) && (rt == Type::Integer || rt == Type::Anything || rt == Type::Unknown) { Type::Integer }
                    else { Type::Number }
                }
                BinOp::Sub | BinOp::Mul | BinOp::Mod => {
                    let lt = Self::infer_expr_type(left);
                    let rt = Self::infer_expr_type(right);
                    if (lt == Type::Integer || lt == Type::Anything || lt == Type::Unknown) && (rt == Type::Integer || rt == Type::Anything || rt == Type::Unknown) { Type::Integer } else { Type::Number }
                }
                BinOp::Pow => {
                    let lt = Self::infer_expr_type(left);
                    let rt = Self::infer_expr_type(right);
                    if lt == Type::Integer && (rt == Type::Integer || rt == Type::Anything || rt == Type::Unknown) {
                        Type::Integer
                    } else {
                        Type::Number
                    }
                }
                BinOp::Div => Type::Number,
                BinOp::And | BinOp::Or => Type::Boolean,
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => Type::Boolean,
            },
            _ => Type::Anything,
        }
    }

    fn collect_module(&mut self, path: &str) {
        let name = path.rsplit('/').next().unwrap_or(path);
        if self.modules.contains_key(name) {
            return;
        }
        let exports: Vec<(String, Type)> = match name {
            "math" => vec![
                ("sin".to_string(), Type::Function(vec![Type::Number], Box::new(Type::Number))),
                ("cos".to_string(), Type::Function(vec![Type::Number], Box::new(Type::Number))),
                ("tan".to_string(), Type::Function(vec![Type::Number], Box::new(Type::Number))),
                ("sqrt".to_string(), Type::Function(vec![Type::Number], Box::new(Type::Number))),
                ("abs".to_string(), Type::Function(vec![Type::Number], Box::new(Type::Number))),
                ("floor".to_string(), Type::Function(vec![Type::Number], Box::new(Type::Number))),
                ("ceil".to_string(), Type::Function(vec![Type::Number], Box::new(Type::Number))),
                ("pi".to_string(), Type::Number),
            ],
            "random" => vec![
                ("random".to_string(), Type::Function(vec![], Box::new(Type::Number))),
                ("seed".to_string(), Type::Function(vec![Type::Integer], Box::new(Type::Nothing))),
            ],
            "string" => vec![
                ("upper".to_string(), Type::Function(vec![Type::String], Box::new(Type::String))),
                ("lower".to_string(), Type::Function(vec![Type::String], Box::new(Type::String))),
                ("trim".to_string(), Type::Function(vec![Type::String], Box::new(Type::String))),
                ("split".to_string(), Type::Function(vec![Type::String, Type::String], Box::new(Type::List(Box::new(Type::String))))),
                ("contains".to_string(), Type::Function(vec![Type::String, Type::String], Box::new(Type::Boolean))),
                ("starts_with".to_string(), Type::Function(vec![Type::String, Type::String], Box::new(Type::Boolean))),
                ("ends_with".to_string(), Type::Function(vec![Type::String, Type::String], Box::new(Type::Boolean))),
                ("replace".to_string(), Type::Function(vec![Type::String, Type::String, Type::String], Box::new(Type::String))),
                ("slice".to_string(), Type::Function(vec![Type::String, Type::Integer], Box::new(Type::String))),
                ("substring".to_string(), Type::Function(vec![Type::String, Type::Integer, Type::Integer], Box::new(Type::String))),
            ],
            "time" => vec![("now".to_string(), Type::Function(vec![], Box::new(Type::Number)))],
            "system" => vec![
                ("run".to_string(), Type::Function(vec![Type::String], Box::new(Type::String))),
                ("open".to_string(), Type::Function(vec![Type::String], Box::new(Type::Nothing))),
                ("alert".to_string(), Type::Function(vec![Type::String], Box::new(Type::Nothing))),
                ("confirm".to_string(), Type::Function(vec![Type::String], Box::new(Type::Boolean))),
                ("notify".to_string(), Type::Function(vec![Type::String, Type::String], Box::new(Type::Nothing))),
            ],
            "path" => vec![
                ("join".to_string(), Type::Function(vec![Type::String, Type::String], Box::new(Type::String))),
                ("basename".to_string(), Type::Function(vec![Type::String], Box::new(Type::String))),
                ("dirname".to_string(), Type::Function(vec![Type::String], Box::new(Type::String))),
                ("extension".to_string(), Type::Function(vec![Type::String], Box::new(Type::String))),
                ("is_absolute".to_string(), Type::Function(vec![Type::String], Box::new(Type::Boolean))),
            ],
            "test" => vec![
                ("assert".to_string(), Type::Function(vec![Type::Boolean], Box::new(Type::Nothing))),
                ("assert_equal".to_string(), Type::Function(vec![Type::Anything, Type::Anything], Box::new(Type::Nothing))),
                ("assert_raises".to_string(), Type::Function(vec![Type::Anything], Box::new(Type::Nothing))),
            ],
            _ => {
                // Try to load an installed package, then a standard-library source
                // file or interface.
                let file_path = crate::package_manager::package_path(name)
                    .or_else(|| crate::semantic::find_stdlib_module(name))
                    .or_else(|| crate::semantic::find_stdlib_interface(name));
                if let Some(file_path) = file_path {
                    if let Ok(source) = std::fs::read_to_string(&file_path) {
                        if let Ok(program) = parse_module_source(&source) {
                            let mut all_exports: Vec<(String, Type)> = Vec::new();
                            let mut explicit_exports: Vec<String> = Vec::new();
                            let mut has_export = false;
                            for stmt in &program.statements {
                                match stmt {
                                    Stmt::Define { name, params, return_type, body, .. } => {
                                        let arg_types: Vec<Type> = params
                                            .iter()
                                            .map(|(_, ann)| ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown))
                                            .collect();
                                        let ret = return_type.as_deref().map(parse_type_ann).unwrap_or_else(|| Self::infer_return_type(body));
                                        all_exports.push((name.clone(), Type::Function(arg_types, Box::new(ret))));
                                    }
                                    Stmt::Class { name, .. } => {
                                        all_exports.push((name.clone(), Type::Class(name.clone())));
                                    }
                                    Stmt::Export(names) => {
                                        has_export = true;
                                        explicit_exports.extend(names.iter().cloned());
                                    }
                                    _ => {}
                                }
                            }
                            if has_export {
                                explicit_exports
                                    .iter()
                                    .filter_map(|n| all_exports.iter().find(|(k, _)| k == n).cloned())
                                    .collect()
                            } else {
                                all_exports
                            }
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            }
        };
        if !exports.is_empty() {
            let map: HashMap<String, Type> = exports.iter().cloned().collect();
            self.modules.insert(name.to_string(), map);
            self.define(name, Type::Module(name.to_string()));
            for (n, ty) in &exports {
                self.define(n, ty.clone());
            }
        }
    }

    fn define(&mut self, name: &str, ty: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    /// Zero-arity functions are auto-called when used as values at runtime.
    /// Reflect that in the static type so `let x be random.` types `x` as
    /// `number` rather than `function`.
    fn auto_call_type(ty: Type) -> Type {
        if let Type::Function(params, ret) = &ty
            && params.is_empty() {
                return *ret.clone();
            }
        ty
    }

    fn error(&mut self, span: &Span, msg: String) {
        self.errors.push((span.clone(), msg));
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, type_ann, value, span } => {
                let value_ty = self.check_expr(value);
                let ann_ty = type_ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown);
                if ann_ty != Type::Anything && ann_ty != Type::Unknown && !value_ty.is_subtype(&ann_ty) {
                    self.error(span, format!("type mismatch: expected '{}', got '{}'", ann_ty.name(), value_ty.name()));
                }
                if ann_ty != Type::Anything && ann_ty != Type::Unknown {
                    self.check_expr_against_ann(value, &ann_ty);
                }
                let bind_ty = if ann_ty == Type::Unknown { value_ty } else { ann_ty };
                self.define(name, bind_ty);
            }
            Stmt::Set { target, value } => {
                let value_ty = self.check_expr(value);
                self.check_assign_target(target, &value_ty);
            }
            Stmt::Show(expr) => {
                self.check_expr(expr);
            }
            Stmt::If { cond, then_branch, else_branch } => {
                let cond_ty = self.check_expr(cond);
                if cond_ty != Type::Anything && cond_ty != Type::Unknown && cond_ty != Type::Boolean && cond_ty != Type::Error {
                    self.error(&cond.span().cloned().unwrap_or(Span { line: 0, col: 0 }), format!("condition must be boolean, got '{}'", cond_ty.name()));
                }
                self.check_block(then_branch);
                self.check_block(else_branch);
            }
            Stmt::While { cond, body } => {
                let cond_ty = self.check_expr(cond);
                if cond_ty != Type::Anything && cond_ty != Type::Unknown && cond_ty != Type::Boolean && cond_ty != Type::Error {
                    self.error(&cond.span().cloned().unwrap_or(Span { line: 0, col: 0 }), format!("while condition must be boolean, got '{}'", cond_ty.name()));
                }
                self.check_block(body);
            }
            Stmt::For { var, iterable, body } => {
                let iter_ty = self.check_expr(iterable);
                match iter_ty {
                    Type::List(_) | Type::Dict(_, _) | Type::Range | Type::String | Type::Anything | Type::Unknown | Type::Error => {}
                    _ => {
                        self.error(&iterable.span().cloned().unwrap_or(Span { line: 0, col: 0 }), format!("cannot iterate over '{}'", iter_ty.name()));
                    }
                }
                self.push_scope();
                let elem_ty = match iter_ty {
                    Type::List(t) => *t,
                    Type::Dict(k, _) => *k,
                    Type::Range => Type::Integer,
                    Type::String => Type::String,
                    _ => Type::Anything,
                };
                self.define(var, elem_ty);
                self.check_block(body);
                self.pop_scope();
            }
            Stmt::Return { value, span } => {
                let value_ty = value.as_ref().map(|e| self.check_expr(e)).unwrap_or(Type::Nothing);
                if let Some(ret) = self.return_types.last()
                    && *ret != Type::Anything && !value_ty.is_subtype(ret) {
                        self.error(span, format!("return type mismatch: expected '{}', got '{}'", ret.name(), value_ty.name()));
                    }
            }
            Stmt::Define { name, params, return_type, body, span, .. } => {
                let arg_types: Vec<Type> = params
                    .iter()
                    .map(|(_, ann)| ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown))
                    .collect();
                let ret = return_type.as_deref().map(parse_type_ann).unwrap_or_else(|| Self::infer_return_type(body));
                // Redefine with inferred function type in case annotations were missing.
                let func_type = Type::Function(arg_types.clone(), Box::new(ret.clone()));
                self.define(name, func_type);
                self.push_scope();
                self.return_types.push(ret.clone());
                for ((p_name, ann), ty) in params.iter().zip(arg_types.iter()) {
                    self.define(p_name, ann.as_deref().map(parse_type_ann).unwrap_or(ty.clone()));
                }
                self.check_block(body);
                if ret != Type::Anything && ret != Type::Unknown && ret != Type::Nothing && ret != Type::Error && !self.block_returns(body) {
                    self.error(span, format!("function '{}' may not return a value on all paths", name));
                }
                self.return_types.pop();
                self.pop_scope();
            }
            Stmt::Class { name, init, methods, .. } => {
                let class_name = name.clone();
                self.push_scope();
                self.define("this", Type::Instance(class_name.clone()));
                if let Some(Init { params, body, .. }) = init {
                    self.push_scope();
                    for (p_name, ann) in params {
                        self.define(p_name, ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown));
                    }
                    self.check_block(body);
                    self.pop_scope();
                }
                for m in methods {
                    if let Stmt::Define { name, params, return_type, body, span, .. } = m {
                        let _arg_types: Vec<Type> = params
                            .iter()
                            .map(|(_, ann)| ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown))
                            .collect();
                        let ret = return_type.as_deref().map(parse_type_ann).unwrap_or_else(|| Self::infer_return_type(body));
                        self.return_types.push(ret.clone());
                        self.push_scope();
                        self.define("this", Type::Instance(class_name.clone()));
                        for (p_name, ann) in params {
                            self.define(p_name, ann.as_deref().map(parse_type_ann).unwrap_or(Type::Unknown));
                        }
                        self.check_block(body);
                        if ret != Type::Anything && ret != Type::Unknown && ret != Type::Nothing && ret != Type::Error && !self.block_returns(body) {
                            self.error(span, format!("method '{}' may not return a value on all paths", name));
                        }
                        self.pop_scope();
                        self.return_types.pop();
                    } else {
                        self.error(&Span { line: 0, col: 0 }, "class methods must be define statements".to_string());
                    }
                }
                self.pop_scope();
            }
            Stmt::Try { body, catch_body, .. } => {
                self.check_block(body);
                self.push_scope();
                self.define("err", Type::Instance("Error".to_string()));
                self.check_block(catch_body);
                self.pop_scope();
            }
            Stmt::Expr(expr) => {
                self.check_expr(expr);
            }
            Stmt::Read { name, .. } => {
                self.define(name, Type::String);
            }
            Stmt::Write { content, path } => {
                self.check_expr(content);
                self.check_expr(path);
            }
            Stmt::Import(paths) => {
                for (path, _) in paths {
                    self.collect_module(path);
                }
            }
            Stmt::Export(_) | Stmt::Pass | Stmt::Init(_) => {}
        }
    }

    fn check_block(&mut self, stmts: &[Stmt]) {
        self.push_scope();
        for stmt in stmts {
            self.check_stmt(stmt);
        }
        self.pop_scope();
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn check_assign_target(&mut self, target: &AssignTarget, value_ty: &Type) {
        match target {
            AssignTarget::Variable { name, span } => {
                if let Some(ty) = self.lookup(name)
                    && ty != Type::Anything && !value_ty.is_subtype(&ty) {
                        self.error(span, format!("assignment type mismatch: expected '{}', got '{}'", ty.name(), value_ty.name()));
                    }
            }
            AssignTarget::Index { object, index, .. } => {
                self.check_expr(object);
                self.check_expr(index);
            }
            AssignTarget::Property { object, name, span } => {
                let obj_ty = self.check_expr(object);
                if let Type::Instance(class_name) = &obj_ty
                    && let Some(info) = self.classes.get_mut(class_name) {
                        if let Some(field_ty) = info.fields.get(name).cloned() {
                            if field_ty == Type::Anything {
                                info.fields.insert(name.clone(), value_ty.clone());
                            } else if !value_ty.is_subtype(&field_ty) {
                                self.error(span, format!("assignment type mismatch: expected '{}', got '{}'", field_ty.name(), value_ty.name()));
                            }
                        } else {
                            self.error(span, format!("class '{}' has no property '{}'", class_name, name));
                        }
                    }
            }
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Type {
        match expr {
            Expr::Integer(_, _) => Type::Integer,
            Expr::Number(_, _) => Type::Number,
            Expr::String(_, _) => Type::String,
            Expr::Bool(_, _) => Type::Boolean,
            Expr::Nothing(_) => Type::Nothing,
            Expr::Variable { name, .. } => {
                // Undefined names are reported by the existing semantic check; here we
                // optimistically treat unknowns as dynamic to avoid duplicate/false errors.
                Self::auto_call_type(self.lookup(name).unwrap_or(Type::Anything))
            }
            Expr::Unary { op, operand, span } => {
                let operand_ty = self.check_expr(operand);
                match op {
                    UnaryOp::Neg => {
                        if operand_ty != Type::Anything && operand_ty != Type::Unknown && operand_ty != Type::Integer && operand_ty != Type::Number && operand_ty != Type::Error {
                            self.error(span, format!("'-' requires a number, got '{}'", operand_ty.name()));
                        }
                        if operand_ty == Type::Integer { Type::Integer } else { Type::Number }
                    }
                    UnaryOp::Not => {
                        if operand_ty != Type::Anything && operand_ty != Type::Unknown && operand_ty != Type::Boolean && operand_ty != Type::Error {
                            self.error(span, format!("'not' requires a boolean, got '{}'", operand_ty.name()));
                        }
                        Type::Boolean
                    }
                }
            }
            Expr::Binary { op, left, right, span } => {
                let left_ty = self.check_expr(left);
                let right_ty = self.check_expr(right);
                // BigInt exponentiation returns an integer when the exponent is a
                // non-negative integer literal; otherwise the result is a number.
                if *op == BinOp::Pow && left_ty == Type::Integer && right_ty == Type::Integer {
                    if let Expr::Integer(n, _) = right.as_ref()
                        && n >= &BigInt::from(0) {
                            return Type::Integer;
                        }
                    return Type::Number;
                }
                self.check_binary(op, &left_ty, &right_ty, span)
            }
            Expr::Call { callee, args, span } => {
                let callee_ty = self.check_expr(callee);
                let arg_types: Vec<Type> = args.iter().map(|a| self.check_expr(a)).collect();
                // Built-in `range` is variadic and requires integer arguments.
                if let Expr::Variable { name, .. } = callee.as_ref()
                    && name == "range" {
                        for (i, got) in arg_types.iter().enumerate() {
                            if *got != Type::Anything && *got != Type::Unknown && *got != Type::Integer && *got != Type::Number && *got != Type::Error {
                                self.error(span, format!("argument {} type mismatch: expected 'integer', got '{}'", i + 1, got.name()));
                            }
                        }
                        return Type::Anything;
                    }
                match callee_ty {
                    Type::Function(params, ret) => {
                        if params.len() != arg_types.len() {
                            self.error(span, format!("expected {} arguments, got {}", params.len(), arg_types.len()));
                        } else {
                            for (i, (expected, got)) in params.iter().zip(arg_types.iter()).enumerate() {
                                if *expected != Type::Anything && !got.is_subtype(expected) {
                                    self.error(span, format!("argument {} type mismatch: expected '{}', got '{}'", i + 1, expected.name(), got.name()));
                                }
                            }
                        }
                        *ret
                    }
                    Type::Anything => Type::Anything,
                    _ => {
                        self.error(span, format!("cannot call '{}'", callee_ty.name()));
                        Type::Error
                    }
                }
            }
            Expr::New { class, args, span } => {
                let class_ty = self.check_expr(class);
                let arg_types: Vec<Type> = args.iter().map(|a| self.check_expr(a)).collect();
                if let Type::Class(name) = class_ty {
                    if let Some(info) = self.classes.get(&name) {
                        let expected = info.init_params.clone();
                        if expected.len() != arg_types.len() {
                            self.error(span, format!("class '{}' init expects {} arguments, got {}", name, expected.len(), arg_types.len()));
                        } else {
                            for (i, (expected, got)) in expected.iter().zip(arg_types.iter()).enumerate() {
                                if *expected != Type::Anything && !got.is_subtype(expected) {
                                    self.error(span, format!("class '{}' init argument {} type mismatch: expected '{}', got '{}'", name, i + 1, expected.name(), got.name()));
                                }
                            }
                        }
                    }
                    Type::Instance(name)
                } else {
                    self.error(span, "new requires a class".to_string());
                    Type::Error
                }
            }
            Expr::Tell { object, method, args, span } => {
                let obj_ty = self.check_expr(object);
                let arg_types: Vec<Type> = args.iter().map(|a| self.check_expr(a)).collect();
                if let Type::Instance(class_name) = &obj_ty {
                    let method_info = self.classes.get(class_name).and_then(|info| info.methods.get(method).cloned());
                    if let Some(Type::Function(params, ret)) = method_info {
                        if params.len() != arg_types.len() {
                            self.error(span, format!("method '{}' expects {} arguments, got {}", method, params.len(), arg_types.len()));
                        } else {
                            for (i, (expected, got)) in params.iter().zip(arg_types.iter()).enumerate() {
                                if *expected != Type::Anything && !got.is_subtype(expected) {
                                    self.error(span, format!("argument {} type mismatch: expected '{}', got '{}'", i + 1, expected.name(), got.name()));
                                }
                            }
                        }
                        return *ret;
                    } else if self.classes.contains_key(class_name) {
                        self.error(span, format!("class '{}' has no method '{}'", class_name, method));
                    } else {
                        self.error(span, format!("unknown class '{}'", class_name));
                    }
                } else if obj_ty != Type::Anything && obj_ty != Type::Error {
                    self.error(span, format!("cannot send message to '{}'", obj_ty.name()));
                }
                Type::Anything
            }
            Expr::Property { object, name, span } => {
                let obj_ty = self.check_expr(object);
                match &obj_ty {
                    Type::Instance(class_name) => {
                        if let Some(info) = self.classes.get(class_name) {
                            if let Some(ty) = info.fields.get(name) {
                                return ty.clone();
                            }
                            if info.methods.contains_key(name) {
                                self.error(span, format!("method '{}' must be called with 'tell <object> to {}'", name, name));
                                return Type::Error;
                            }
                        }
                        if class_name == "Error" {
                            return match name.as_str() {
                                "message" => Type::String,
                                "line" | "col" => Type::Integer,
                                _ => Type::Anything,
                            };
                        }
                        self.error(span, format!("class '{}' has no property '{}'", class_name, name));
                        Type::Error
                    }
                    Type::Anything | Type::Error => Type::Anything,
                    _ => {
                        self.error(span, format!("cannot access property on '{}'", obj_ty.name()));
                        Type::Error
                    }
                }
            }
            Expr::Qualified { name, module, span: _ } => {
                // Built-in modules have known signatures; local modules are treated as dynamic.
                if let Some(mod_map) = self.modules.get(module)
                    && let Some(ty) = mod_map.get(name) {
                        return Self::auto_call_type(ty.clone());
                    }
                Type::Anything
            }
            Expr::Index { object, index, span } => {
                let obj_ty = self.check_expr(object);
                let idx_ty = self.check_expr(index);
                match &obj_ty {
                    Type::List(_) | Type::String => {
                        if idx_ty != Type::Anything && idx_ty != Type::Unknown && idx_ty != Type::Integer && idx_ty != Type::Error {
                            self.error(&index.span().cloned().unwrap_or(Span { line: 0, col: 0 }), format!("index must be integer, got '{}'", idx_ty.name()));
                        }
                    }
                    Type::Dict(k, _) => {
                        if idx_ty != Type::Anything && idx_ty != Type::Unknown && idx_ty != Type::Error && !idx_ty.is_subtype(k) {
                            self.error(&index.span().cloned().unwrap_or(Span { line: 0, col: 0 }), format!("dictionary key type mismatch: expected '{}', got '{}'", k.name(), idx_ty.name()));
                        }
                    }
                    Type::Anything | Type::Unknown | Type::Error => {}
                    _ => {
                        self.error(span, format!("cannot index into '{}'", obj_ty.name()));
                    }
                }
                match obj_ty {
                    Type::List(t) => *t,
                    Type::Dict(_, v) => *v,
                    Type::String => Type::String,
                    Type::Anything | Type::Unknown | Type::Error => Type::Anything,
                    _ => Type::Error,
                }
            }
            Expr::List(elems, _span) => {
                if elems.is_empty() {
                    return Type::List(Box::new(Type::Anything));
                }
                // Unannotated lists may be heterogeneous (like dictionaries). Infer
                // the common type when all elements agree, otherwise Anything.
                let mut ty = Type::Anything;
                for e in elems {
                    let et = self.check_expr(e);
                    ty = Self::merge_types(&ty, &et);
                }
                Type::List(Box::new(ty))
            }
            Expr::Dict(pairs, _span) => {
                if pairs.is_empty() {
                    return Type::Dict(Box::new(Type::Anything), Box::new(Type::Anything));
                }
                // Period dictionaries are heterogeneous at runtime. We infer the
                // common key/value type when all elements agree, otherwise fall
                // back to Anything so unannotated mixed dictionaries are allowed.
                let mut kty = Type::Anything;
                let mut vty = Type::Anything;
                for (k, v) in pairs {
                    let kt = self.check_expr(k);
                    let vt = self.check_expr(v);
                    kty = Self::merge_types(&kty, &kt);
                    vty = Self::merge_types(&vty, &vt);
                }
                Type::Dict(Box::new(kty), Box::new(vty))
            }
            Expr::Ellipsis => Type::Anything,
        }
    }

    fn merge_types(a: &Type, b: &Type) -> Type {
        if *a == Type::Anything || *a == Type::Unknown { return b.clone(); }
        if *b == Type::Anything || *b == Type::Unknown { return a.clone(); }
        if a.is_subtype(b) { return b.clone(); }
        if b.is_subtype(a) { return a.clone(); }
        Type::Error
    }

    /// Check a literal expression against an explicit annotation. Used for
    /// list/dict literals where the inferred type would otherwise be Anything.
    fn check_expr_against_ann(&mut self, expr: &Expr, ann: &Type) {
        match (expr, ann) {
            (Expr::List(elems, span), Type::List(elem_ann)) => {
                for e in elems {
                    let et = self.check_expr(e);
                    if !et.is_subtype(elem_ann) {
                        self.error(span, format!("list element type mismatch: expected '{}', got '{}'", elem_ann.name(), et.name()));
                    }
                }
            }
            (Expr::Dict(pairs, span), Type::Dict(key_ann, val_ann)) => {
                for (k, v) in pairs {
                    let kt = self.check_expr(k);
                    let vt = self.check_expr(v);
                    if !kt.is_subtype(key_ann) {
                        self.error(span, format!("dictionary key type mismatch: expected '{}', got '{}'", key_ann.name(), kt.name()));
                    }
                    if !vt.is_subtype(val_ann) {
                        self.error(span, format!("dictionary value type mismatch: expected '{}', got '{}'", val_ann.name(), vt.name()));
                    }
                }
            }
            _ => {}
        }
    }

    fn check_binary(&mut self, op: &BinOp, left: &Type, right: &Type, span: &Span) -> Type {
        let is_numeric = |t: &Type| matches!(t, Type::Integer | Type::Number | Type::Anything | Type::Unknown | Type::Error);
        let is_string = |t: &Type| matches!(t, Type::String | Type::Anything | Type::Unknown | Type::Error);
        let is_boolean = |t: &Type| matches!(t, Type::Boolean | Type::Anything | Type::Unknown | Type::Error);
        let is_list = |t: &Type| matches!(t, Type::List(_) | Type::Anything | Type::Unknown | Type::Error);
        let is_string_factor = |t: &Type| matches!(t, Type::Integer | Type::Number | Type::Anything | Type::Unknown | Type::Error);
        let both_integer_like = |l: &Type, r: &Type| {
            (matches!(l, Type::Integer | Type::Anything | Type::Unknown | Type::Error))
                && (matches!(r, Type::Integer | Type::Anything | Type::Unknown | Type::Error))
        };
        match op {
            BinOp::Add => {
                if is_numeric(left) && is_numeric(right) {
                    if both_integer_like(left, right) { Type::Integer } else { Type::Number }
                } else if is_string(left) && is_string(right) {
                    Type::String
                } else if is_list(left) && is_list(right) {
                    Type::List(Box::new(Type::Anything))
                } else {
                    self.error(span, format!("invalid operands for '+': '{}' and '{}'", left.name(), right.name()));
                    Type::Error
                }
            }
            BinOp::Mul => {
                if (left == &Type::String && is_string_factor(right))
                    || (right == &Type::String && is_string_factor(left))
                {
                    Type::String
                } else if is_numeric(left) && is_numeric(right) {
                    if both_integer_like(left, right) { Type::Integer } else { Type::Number }
                } else {
                    self.error(span, format!("invalid operands for '*': '{}' and '{}'", left.name(), right.name()));
                    Type::Error
                }
            }
            BinOp::Sub | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                if is_numeric(left) && is_numeric(right) {
                    if op == &BinOp::Sub || op == &BinOp::Mod || op == &BinOp::Pow {
                        if both_integer_like(left, right) { Type::Integer } else { Type::Number }
                    } else {
                        Type::Number
                    }
                } else {
                    self.error(span, format!("invalid operands for '{}': '{}' and '{}'", op_name(op), left.name(), right.name()));
                    Type::Error
                }
            }
            BinOp::And | BinOp::Or => {
                if is_boolean(left) && is_boolean(right) {
                    Type::Boolean
                } else {
                    self.error(span, format!("'{}' requires booleans, got '{}' and '{}'", op_name(op), left.name(), right.name()));
                    Type::Error
                }
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                Type::Boolean
            }
        }
    }
}

impl TypeChecker {
    fn infer_init_field_type(expr: &Expr, param_types: &HashMap<String, Type>) -> Type {
        match expr {
            Expr::Integer(_, _) => Type::Integer,
            Expr::Number(_, _) => Type::Number,
            Expr::String(_, _) => Type::String,
            Expr::Bool(_, _) => Type::Boolean,
            Expr::Nothing(_) => Type::Nothing,
            Expr::Variable { name, .. } => param_types.get(name).cloned().unwrap_or(Type::Anything),
            Expr::New { class, .. } => {
                if let Expr::Variable { name, .. } = class.as_ref() {
                    Type::Instance(name.clone())
                } else {
                    Type::Anything
                }
            }
            _ => Type::Anything,
        }
    }

    fn block_returns(&self, stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            if self.stmt_returns(stmt) {
                return true;
            }
        }
        false
    }

    fn stmt_returns(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Return { .. } => true,
            Stmt::If { cond, then_branch, else_branch } => {
                if self.block_returns(then_branch) && self.block_returns(else_branch) {
                    return true;
                }
                if let Expr::Bool(true, _) = cond
                    && self.block_returns(then_branch) {
                        return true;
                    }
                false
            }
            Stmt::While { cond: Expr::Bool(true, _), body } => {
                self.block_returns(body)
            }
            Stmt::While { .. } => false,
            Stmt::Try { body, catch_body, .. } => {
                self.block_returns(body) && self.block_returns(catch_body)
            }
            _ => false,
        }
    }
}

fn parse_module_source(source: &str) -> Result<Program, Vec<String>> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token().map_err(|e| vec![e])?;
        let eof = matches!(t.kind, crate::lexer::TokenKind::Eof);
        tokens.push(t);
        if eof {
            break;
        }
    }
    Parser::new(tokens).parse_program()
}

fn op_name(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "**",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn check(source: &str) -> Vec<(Span, String)> {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let t = lexer.next_token().expect("lexer should produce a token");
            let eof = matches!(t.kind, crate::lexer::TokenKind::Eof);
            tokens.push(t);
            if eof { break; }
        }
        let program = Parser::new(tokens).parse_program().expect("source should parse");
        TypeChecker::new().check(&program).0
    }

    #[test]
    fn no_errors_for_valid_program() {
        assert!(check("define add with number a, number b returns number:\n    return a + b.\nshow add with 1, 2.").is_empty());
    }

    #[test]
    fn catches_argument_type_mismatch() {
        let errors = check("define add with number a, number b returns number:\n    return a + b.\nshow add with 1, \"two\".");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("type mismatch"));
    }

    #[test]
    fn catches_return_type_mismatch() {
        let errors = check("define f with number x returns number:\n    return \"hello\".");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("return type mismatch"));
    }

    #[test]
    fn catches_annotated_list_element_mismatch() {
        let errors = check("let xs be list of integer [1, \"a\"].");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("type mismatch"));
    }

    #[test]
    fn unannotated_mixed_list_is_allowed() {
        assert!(check("let xs be [1, \"a\"].").is_empty());
    }

    #[test]
    fn integer_plus_number_is_number() {
        let errors = check("show 1 + 2.5.");
        assert!(errors.is_empty());
    }

    #[test]
    fn boolean_condition_required() {
        let errors = check("if 1 then:\n    show \"yes\".");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("condition must be boolean"));
    }

    #[test]
    fn index_type_error_points_at_index_literal() {
        let errors = check("let xs be [1, 2, 3].\nshow xs[\"a\"].");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("index must be integer"));
        assert_eq!(errors[0].0.line, 2);
        assert_eq!(errors[0].0.col, 9); // opening quote of "a"
    }

    #[test]
    fn class_field_initialized_in_init_body_is_visible() {
        let errors = check("class A:\n    init:\n        set the x of this to 0.\nlet a be new A.\nshow the x of a.");
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn property_assignment_type_mismatch_is_caught() {
        let errors = check("class A:\n    init with number x:\n        set the x of this to x.\nlet a be new A with 5.\nset the x of a to \"hello\".");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("assignment type mismatch"), "{:?}", errors);
    }

    #[test]
    fn method_accessed_as_property_is_rejected() {
        // Methods must be called with 'tell'; accessing a method as a property
        // is a static error.
        let errors = check("class Person:\n    init with string name:\n        set the name of this to name.\n    define greet returns string:\n        return \"Hi, \" + the name of this.\nlet p be new Person with \"Ada\".\nshow the greet of p.");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("method 'greet' must be called with 'tell"), "{:?}", errors);
    }

    #[test]
    fn missing_return_is_caught() {
        let errors = check("define f returns number:\n    show 1.");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("may not return a value on all paths"), "{:?}", errors);
    }

    #[test]
    fn if_else_return_satisfies_return_analysis() {
        let errors = check("define f returns number:\n    if true then:\n        return 1.\n    otherwise:\n        return 2.");
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn while_true_return_satisfies_return_analysis() {
        let errors = check("define f returns number:\n    while true repeat:\n        return 1.");
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn infers_nothing_for_function_without_return() {
        // No return statement -> inferred as nothing. The function is used where
        // a string is expected, so the call site should fail.
        let errors = check("define greet with string name:\n    show name.\nlet s be string greet with \"Ada\".");
        assert_eq!(errors.len(), 1, "{:?}", errors);
        assert!(errors[0].1.contains("type mismatch"), "{:?}", errors);
    }

    #[test]
    fn infers_literal_return_type() {
        // No annotation, but always returns an integer literal -> inferred as integer.
        let errors = check("define double with x:\n    return x * 2.\nshow double with 5.");
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn infers_string_return_type_from_literal() {
        let errors = check("define greeting:\n    return \"hello\".\nlet s be string greeting.");
        assert!(errors.is_empty(), "{:?}", errors);
    }
}

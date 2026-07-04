use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use num_bigint::BigInt;
use num_traits::cast::{FromPrimitive, ToPrimitive};

use crate::ast::*;
use crate::builtins::{install_builtins, make_math_module, make_random_module, make_string_module, make_time_module};
use crate::environment::Environment;
use crate::lexer::{Lexer, TokenKind};
use crate::parser::Parser;
use crate::value::{range_len, Value};

#[derive(Debug)]
pub enum Control {
    Return(Value, Span),
    Error(String),
    RuntimeError(String, Span),
}

pub struct Interpreter {
    env: Rc<RefCell<Environment>>,
    pub output: Vec<String>,
    modules: RefCell<HashMap<String, Rc<RefCell<Environment>>>>,
    /// Modules currently being loaded; used to detect circular imports.
    loading_modules: RefCell<HashSet<String>>,
    silent: bool,
    current_path: Option<PathBuf>,
    /// True while interpreting a module file; marks top-level functions/classes
    /// as originating from a module so their runtime errors can be mapped to
    /// the user's call site.
    loading_module: bool,
}

impl Interpreter {
    pub fn new() -> Self {
        let env = Environment::new();
        install_builtins(&env.borrow());
        Self { env, output: Vec::new(), modules: RefCell::new(HashMap::new()), loading_modules: RefCell::new(HashSet::new()), silent: false, current_path: None, loading_module: false }
    }

    pub fn set_current_path(&mut self, path: impl Into<PathBuf>) {
        self.current_path = Some(path.into());
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path);
        if p.is_absolute() { return p; }
        if let Some(current) = &self.current_path {
            let dir = if current.is_file() { current.parent().unwrap_or(current) } else { current };
            return dir.join(p);
        }
        p
    }

    pub fn interpret(&mut self, program: &Program) -> Result<(), Control> {
        for stmt in &program.statements {
            self.execute(stmt)?;
        }
        Ok(())
    }

    /// Check that `value` satisfies the given type annotation. `integer` is a
    /// subset of `number`; class names match instances of that class; compound
    /// types are checked recursively.
    fn check_type(&self, value: &Value, ann: &str, span: &Span) -> Result<(), Control> {
        let parts: Vec<&str> = ann.split_whitespace().collect();
        if parts.is_empty() { return Ok(()); }
        let ok = match parts[0] {
            "nothing" => matches!(value, Value::Nothing),
            "boolean" => matches!(value, Value::Bool(_)),
            "integer" => matches!(value, Value::Integer(_)) || matches!(value, Value::Number(n) if n.fract() == 0.0),
            "number" => matches!(value, Value::Integer(_) | Value::Number(_)),
            "string" => matches!(value, Value::String(_)),
            "list" => {
                if let Value::List(list) = value {
                    if parts.len() >= 3 && parts[1] == "of" {
                        let elem_ann = parts[2..].join(" ");
                        list.borrow().iter().all(|item| self.check_type(item, &elem_ann, span).is_ok())
                    } else { true }
                } else { false }
            }
            "dictionary" => {
                if let Value::Dict(dict) = value {
                    if parts.len() >= 5 && parts[1] == "of" && parts[3] == "to" {
                        let key_ann = parts[2];
                        let val_ann = parts[4..].join(" ");
                        dict.borrow().iter().all(|(k, v)| {
                            self.check_type(&k.to_value(), key_ann, span).is_ok()
                                && self.check_type(v, &val_ann, span).is_ok()
                        })
                    } else { true }
                } else { false }
            }
            "function" => matches!(value, Value::Function { .. } | Value::BuiltIn { .. }),
            "class" => matches!(value, Value::Class { .. }),
            class_name => {
                if let Value::Instance { class, .. } = value {
                    if let Value::Class { name, .. } = class.as_ref() { name == class_name } else { false }
                } else { false }
            }
        };
        if ok {
            Ok(())
        } else {
            Err(Control::RuntimeError(
                format!("Type mismatch: expected '{}', got '{}'", ann, value.type_name()),
                span.clone(),
            ))
        }
    }

    fn execute(&mut self, stmt: &Stmt) -> Result<(), Control> {
        match stmt {
            Stmt::Let { name, type_ann, value, span } => {
                let v = self.evaluate(value)?;
                if let Some(ann) = type_ann {
                    if let Some(value_span) = value.span() {
                        self.check_type(&v, ann, value_span)?;
                    } else {
                        self.check_type(&v, ann, span)?;
                    }
                }
                self.env.borrow().define(name, v, type_ann.clone());
            }
            Stmt::Set { target, value } => {
                let v = self.evaluate(value)?;
                self.assign_target(target, v)?;
            }
            Stmt::Show(expr) => {
                let v = self.evaluate(expr)?;
                let text = v.to_string();
                self.output.push(text.clone());
                if !self.silent {
                    println!("{}", text);
                }
            }
            Stmt::Read { name, path } => {
                let path_str = self.evaluate(path)?.to_string();
                let full_path = self.resolve_path(&path_str);
                match std::fs::read_to_string(&full_path) {
                    Ok(text) => { self.env.borrow().define_untyped(name, Value::String(text)); }
                    Err(e) => return Err(Control::Error(format!("Could not read file '{}': {}", path_str, e))),
                }
            }
            Stmt::Write { content, path } => {
                let content_str = self.evaluate(content)?.to_string();
                let path_str = self.evaluate(path)?.to_string();
                let full_path = self.resolve_path(&path_str);
                if let Err(e) = std::fs::write(&full_path, content_str) {
                    return Err(Control::Error(format!("Could not write file '{}': {}", path_str, e)));
                }
            }
            Stmt::Try { body, catch_var, catch_body } => {
                match self.execute_block(body) {
                    Ok(()) => {}
                    Err(Control::Error(msg)) => {
                        let err = Value::Error { message: msg, line: 0, col: 0 };
                        let env = Environment::with_parent(self.env.clone());
                        env.borrow().define_untyped(catch_var, err);
                        let old = self.env.clone();
                        self.env = env;
                        let result = self.execute_block(catch_body);
                        self.env = old;
                        result?;
                    }
                    Err(Control::RuntimeError(msg, span)) => {
                        let err = Value::Error { message: msg, line: span.line as i64, col: span.col as i64 };
                        let env = Environment::with_parent(self.env.clone());
                        env.borrow().define_untyped(catch_var, err);
                        let old = self.env.clone();
                        self.env = env;
                        let result = self.execute_block(catch_body);
                        self.env = old;
                        result?;
                    }
                    Err(other) => return Err(other),
                }
            }
            Stmt::Export(names) => {
                for name in names { self.env.borrow().add_export(name); }
            }
            Stmt::If { cond, then_branch, else_branch } => {
                if Self::is_truthy(&self.evaluate(cond)?) {
                    self.execute_block(then_branch)?;
                } else if !else_branch.is_empty() {
                    self.execute_block(else_branch)?;
                }
            }
            Stmt::While { cond, body } => {
                while Self::is_truthy(&self.evaluate(cond)?) {
                    self.execute_block(body)?;
                }
            }
            Stmt::For { var, iterable, body } => {
                let iter_value = self.evaluate(iterable)?;
                match iter_value {
                    Value::Range { start, stop, step } => {
                        let mut i = start;
                        while (step > 0 && i < stop) || (step < 0 && i > stop) {
                            let env = Environment::with_parent(self.env.clone());
                            env.borrow().define_untyped(var, Value::Integer(BigInt::from(i)));
                            let old = self.env.clone();
                            self.env = env;
                            let result = self.execute_block(body);
                            self.env = old;
                            result?;
                            i += step;
                        }
                    }
                    _ => {
                        let items = self.iterable_items(&iter_value)?;
                        for item in items {
                            let env = Environment::with_parent(self.env.clone());
                            env.borrow().define_untyped(var, item);
                            let old = self.env.clone();
                            self.env = env;
                            let result = self.execute_block(body);
                            self.env = old;
                            result?;
                        }
                    }
                }
            }
            Stmt::Return { value, span } => {
                let v = match value { Some(e) => self.evaluate(e)?, None => Value::Nothing };
                return Err(Control::Return(v, span.clone()));
            }
            Stmt::Define { name, params, return_type, body, span, .. } => {
                let func = Value::Function {
                    name: name.clone(),
                    params: params.clone(),
                    return_type: return_type.clone(),
                    body: body.clone(),
                    closure: self.env.clone(),
                    span: span.clone(),
                    from_module: self.loading_module,
                };
                self.env.borrow().define_untyped(name, func);
            }
            Stmt::Init(_) => {}
            Stmt::Class { name, init, methods, .. } => {
                let mut method_map = HashMap::new();
                for m in methods {
                    if let Stmt::Define { name: mname, params, return_type, body, span, .. } = m {
                        method_map.insert(mname.clone(), Stmt::Define { name: mname.clone(), params: params.clone(), return_type: return_type.clone(), docstring: None, body: body.clone(), span: span.clone() });
                    }
                }
                self.env.borrow().define_untyped(name, Value::Class { name: name.clone(), init: init.clone(), methods: method_map, from_module: self.loading_module });
            }
            Stmt::Import(paths) => {
                for (path, span) in paths { self.import_module(path, span)?; }
            }
            Stmt::Expr(expr) => { self.evaluate(expr)?; }
            Stmt::Pass => {}
        }
        Ok(())
    }

    fn execute_block(&mut self, stmts: &[Stmt]) -> Result<(), Control> {
        let env = Environment::with_parent(self.env.clone());
        let old = self.env.clone();
        self.env = env;
        for stmt in stmts {
            if let Err(ctrl) = self.execute(stmt) {
                self.env = old;
                return Err(ctrl);
            }
        }
        self.env = old;
        Ok(())
    }

    fn assign_target(&mut self, target: &AssignTarget, value: Value) -> Result<(), Control> {
        match target {
            AssignTarget::Variable { name, span } => {
                if let Some(Some(ann)) = self.env.borrow().get_type(name) {
                    self.check_type(&value, &ann, span)?;
                }
                self.env.borrow().set(name, value).map_err(Control::Error)?;
            }
            AssignTarget::Index { object, index, span } => {
                let obj = self.evaluate(object)?;
                let idx = self.evaluate(index)?;
                match obj {
                    Value::List(list) => {
                        let i = self.as_index(&idx, list.borrow().len(), span)?;
                        list.borrow_mut()[i] = value;
                    }
                    Value::Dict(dict) => {
                        let key = idx.as_key().map_err(|m| Control::RuntimeError(m, span.clone()))?;
                        dict.borrow_mut().insert(key, value);
                    }
                    _ => return Err(Control::RuntimeError(format!("Cannot index into {}", obj.type_name()), span.clone())),
                }
            }
            AssignTarget::Property { object, name, span } => {
                let obj = self.evaluate(object)?;
                if let Value::Instance { fields, .. } = obj {
                    fields.borrow_mut().insert(name.clone(), value);
                } else {
                    return Err(Control::RuntimeError(format!("Cannot set property on {}", obj.type_name()), span.clone()));
                }
            }
        }
        Ok(())
    }

    fn evaluate(&mut self, expr: &Expr) -> Result<Value, Control> {
        match expr {
            Expr::Integer(n, _) => Ok(Value::Integer(n.clone())),
            Expr::Number(n, _) => Ok(Value::Number(*n)),
            Expr::String(s, _) => Ok(Value::String(s.clone())),
            Expr::Bool(b, _) => Ok(Value::Bool(*b)),
            Expr::Nothing(_) => Ok(Value::Nothing),
            Expr::Ellipsis => Ok(Value::Nothing),
            Expr::Variable { name, span } => {
                let value = self.env.borrow().get(name).ok_or_else(|| Control::RuntimeError(format!("Undefined variable '{}'", name), span.clone()))?;
                // Zero-argument functions (like input or random) are called automatically when used as a value.
                if let Value::BuiltIn { min_arity: 0, max_arity: 0, .. } = &value {
                    return self.call_value(&value, vec![], span);
                }
                if let Value::Function { params, .. } = &value
                    && params.is_empty() {
                        return self.call_value(&value, vec![], span);
                    }
                Ok(value)
            }
            Expr::Binary { op, left, right, span } => {
                let l = self.evaluate(left)?;
                // short-circuit boolean operators with runtime type checks
                match op {
                    BinOp::And => {
                        if !matches!(l, Value::Bool(_)) {
                            return Err(Control::RuntimeError("'and' requires boolean operands".to_string(), span.clone()));
                        }
                        if !Self::is_truthy(&l) { return Ok(l); }
                        let r = self.evaluate(right)?;
                        if !matches!(r, Value::Bool(_)) {
                            return Err(Control::RuntimeError("'and' requires boolean operands".to_string(), span.clone()));
                        }
                        return Ok(r);
                    }
                    BinOp::Or => {
                        if !matches!(l, Value::Bool(_)) {
                            return Err(Control::RuntimeError("'or' requires boolean operands".to_string(), span.clone()));
                        }
                        if Self::is_truthy(&l) { return Ok(l); }
                        let r = self.evaluate(right)?;
                        if !matches!(r, Value::Bool(_)) {
                            return Err(Control::RuntimeError("'or' requires boolean operands".to_string(), span.clone()));
                        }
                        return Ok(r);
                    }
                    _ => {}
                }
                let r = self.evaluate(right)?;
                self.eval_binary(op, l, r, span)
            }
            Expr::Unary { op, operand, span } => {
                let v = self.evaluate(operand)?;
                match op {
                    UnaryOp::Neg => self.eval_neg(v, span),
                    UnaryOp::Not => {
                        if let Value::Bool(b) = v {
                            Ok(Value::Bool(!b))
                        } else {
                            Err(Control::RuntimeError("'not' requires a boolean operand".to_string(), span.clone()))
                        }
                    }
                }
            }
            Expr::Call { callee, args, span } => {
                let c = self.evaluate(callee)?;
                let a: Result<Vec<_>, _> = args.iter().map(|e| self.evaluate(e)).collect();
                self.call_value(&c, a?, span)
            }
            Expr::Index { object, index, span } => {
                let obj = self.evaluate(object)?;
                let idx = self.evaluate(index)?;
                match obj {
                    Value::List(list) => {
                        let len = list.borrow().len();
                        if len == 0 {
                            return Err(Control::RuntimeError("Index out of range (list is empty)".to_string(), span.clone()));
                        }
                        let i = self.as_index(&idx, len, span)?;
                        Ok(list.borrow()[i].clone())
                    }
                    Value::Dict(dict) => {
                        let key = idx.as_key().map_err(|m| Control::RuntimeError(m, span.clone()))?;
                        dict.borrow().get(&key).cloned().ok_or_else(|| Control::RuntimeError("Key not found".to_string(), span.clone()))
                    }
                    Value::String(s) => {
                        let i = self.as_index(&idx, s.len(), span)?;
                        Ok(Value::String(s.chars().nth(i).unwrap().to_string()))
                    }
                    Value::Range { start, stop, step } => {
                        let len = range_len(start, stop, step);
                        let i = self.as_index(&idx, len as usize, span)?;
                        Ok(Value::Integer(BigInt::from(start + step * (i as i64))))
                    }
                    _ => Err(Control::RuntimeError(format!("Cannot index into {}", obj.type_name()), span.clone())),
                }
            }
            Expr::Property { object, name, span } => {
                let obj = self.evaluate(object)?;
                match &obj {
                    Value::Instance { class, fields } => {
                        if let Some(v) = fields.borrow().get(name).cloned() {
                            return Ok(v);
                        }
                        if let Value::Class { methods, .. } = class.as_ref()
                            && methods.contains_key(name) {
                                return Err(Control::RuntimeError(
                                    format!("method '{}' must be called with 'tell <object> to {}'", name, name),
                                    span.clone(),
                                ));
                            }
                        Err(Control::RuntimeError(format!("Instance has no property '{}'", name), span.clone()))
                    }
                    Value::Module { env, .. } => {
                        env.borrow().get(name).ok_or_else(|| Control::RuntimeError(format!("Module has no property '{}'", name), span.clone()))
                    }
                    Value::Error { message, line, col } => {
                        match name.as_str() {
                            "message" => Ok(Value::String(message.clone())),
                            "line" => Ok(Value::Integer(BigInt::from(*line))),
                            "col" => Ok(Value::Integer(BigInt::from(*col))),
                            _ => Err(Control::RuntimeError(format!("Error has no property '{}'", name), span.clone())),
                        }
                    }
                    _ => Err(Control::RuntimeError(format!("Cannot access property on {}", obj.type_name()), span.clone())),
                }
            }
            Expr::New { class, args, span } => {
                let cls = self.evaluate(class)?;
                let a: Result<Vec<_>, _> = args.iter().map(|e| self.evaluate(e)).collect();
                self.new_instance(&cls, a?, span)
            }
            Expr::Tell { object, method, args, span } => {
                let obj = self.evaluate(object)?;
                let a: Result<Vec<_>, _> = args.iter().map(|e| self.evaluate(e)).collect();
                self.call_method(&obj, method, a?, span)
            }
            Expr::Qualified { name, module, span } => {
                let mod_env = self.modules.borrow().get(module).cloned()
                    .ok_or_else(|| Control::RuntimeError(format!("Module '{}' not imported", module), span.clone()))?;
                let value = mod_env.borrow().get(name).ok_or_else(|| Control::RuntimeError(format!("'{}' not found in module '{}'", name, module), span.clone()))?;
                // Zero-argument functions are auto-called when used as values, just like
                // unqualified variable references.
                if let Value::BuiltIn { min_arity: 0, max_arity: 0, .. } = &value {
                    return self.call_value(&value, vec![], span);
                }
                if let Value::Function { params, .. } = &value
                    && params.is_empty() {
                        return self.call_value(&value, vec![], span);
                    }
                Ok(value)
            }
            Expr::List(elems, _) => {
                let mut vals = Vec::new();
                for e in elems { vals.push(self.evaluate(e)?); }
                Ok(Value::List(Rc::new(RefCell::new(vals))))
            }
            Expr::Dict(pairs, _) => {
                let mut map = HashMap::new();
                for (k, v) in pairs {
                    let key = self.evaluate(k)?.as_key().map_err(Control::Error)?;
                    map.insert(key, self.evaluate(v)?);
                }
                Ok(Value::Dict(Rc::new(RefCell::new(map))))
            }
        }
    }

    fn eval_binary(&self, op: &BinOp, left: Value, right: Value, span: &Span) -> Result<Value, Control> {
        match op {
            BinOp::Add => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a + b)),
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
                (Value::Integer(a), Value::Number(b)) => Ok(Value::Number(a.to_f64().unwrap_or(0.0) + b)),
                (Value::Number(a), Value::Integer(b)) => Ok(Value::Number(a + b.to_f64().unwrap_or(0.0))),
                (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
                (Value::List(a), Value::List(b)) => {
                    let mut items = a.borrow().clone();
                    items.extend(b.borrow().iter().cloned());
                    Ok(Value::List(Rc::new(RefCell::new(items))))
                }
                _ => Err(Control::RuntimeError("Invalid operands for +".to_string(), span.clone())),
            },
            BinOp::Sub => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a - b)),
                _ => self.numeric_op(&left, &right, |a,b| a - b, |a,b| a.to_f64().unwrap_or(0.0) - b.to_f64().unwrap_or(0.0), span),
            },
            BinOp::Mul => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a * b)),
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
                (Value::Integer(a), Value::Number(b)) => Ok(Value::Number(a.to_f64().unwrap_or(0.0) * b)),
                (Value::Number(a), Value::Integer(b)) => Ok(Value::Number(a * b.to_f64().unwrap_or(0.0))),
                (Value::String(s), Value::Integer(n)) | (Value::Integer(n), Value::String(s)) => {
                    if *n < BigInt::from(0) {
                        return Err(Control::RuntimeError("Cannot repeat string a negative number of times".to_string(), span.clone()));
                    }
                    let count = n.to_usize().ok_or_else(|| Control::RuntimeError("Cannot repeat string: count is too large".to_string(), span.clone()))?;
                    Ok(Value::String(s.repeat(count)))
                }
                _ => Err(Control::RuntimeError("Invalid operands for *".to_string(), span.clone())),
            },
            BinOp::Div => {
                if self.is_zero(&right) {
                    return Err(Control::RuntimeError("Division by zero.".to_string(), span.clone()));
                }
                self.numeric_op(&left, &right, |a,b| a / b, |a,b| a.to_f64().unwrap_or(0.0) / b.to_f64().unwrap_or(0.0), span)
            }
            BinOp::Mod => {
                if self.is_zero(&right) {
                    return Err(Control::RuntimeError("Modulo by zero.".to_string(), span.clone()));
                }
                match (&left, &right) {
                    (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a % b)),
                    _ => self.numeric_op(&left, &right, |a,b| a % b, |a,b| a.to_f64().unwrap_or(0.0) % b.to_f64().unwrap_or(0.0), span),
                }
            }
            BinOp::Pow => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => {
                    if *b < BigInt::from(0) {
                        if *a == BigInt::from(0) {
                            return Err(Control::RuntimeError("Division by zero.".to_string(), span.clone()));
                        }
                        return Ok(Value::Number(a.to_f64().unwrap_or(0.0).powf(b.to_f64().unwrap_or(0.0))));
                    }
                    let exp = b.to_u32().ok_or_else(|| Control::RuntimeError("Exponent too large".to_string(), span.clone()))?;
                    Ok(Value::Integer(a.pow(exp)))
                }
                _ => self.numeric_op(&left, &right, |a,b| a.powf(b), |a,b| a.to_f64().unwrap_or(0.0).powf(b.to_f64().unwrap_or(0.0)), span),
            },
            BinOp::Eq => Ok(Value::Bool(left == right)),
            BinOp::Ne => Ok(Value::Bool(left != right)),
            BinOp::Lt => self.compare(&left, &right, |a,b| a < b, span),
            BinOp::Gt => self.compare(&left, &right, |a,b| a > b, span),
            BinOp::Le => self.compare(&left, &right, |a,b| a <= b, span),
            BinOp::Ge => self.compare(&left, &right, |a,b| a >= b, span),
            _ => unreachable!(),
        }
    }

    fn is_zero(&self, value: &Value) -> bool {
        match value {
            Value::Integer(n) => *n == BigInt::from(0),
            Value::Number(n) => *n == 0.0,
            _ => false,
        }
    }

    fn numeric_op<FN, FI>(&self, left: &Value, right: &Value, float_op: FN, int_to_float: FI, span: &Span) -> Result<Value, Control>
    where FN: Fn(f64, f64) -> f64, FI: Fn(&BigInt, &BigInt) -> f64 {
        match (left, right) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Number(int_to_float(a, b))),
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(float_op(*a, *b))),
            (Value::Integer(a), Value::Number(b)) => Ok(Value::Number(float_op(a.to_f64().unwrap_or(0.0), *b))),
            (Value::Number(a), Value::Integer(b)) => Ok(Value::Number(float_op(*a, b.to_f64().unwrap_or(0.0)))),
            _ => Err(Control::RuntimeError("Operands must be numbers".to_string(), span.clone())),
        }
    }

    fn compare<F>(&self, left: &Value, right: &Value, op: F, span: &Span) -> Result<Value, Control>
    where F: Fn(f64, f64) -> bool {
        // Compare an integer and a finite float, returning the ordering of
        // `integer` relative to `number`.
        fn cmp_integer_number(integer: &BigInt, number: f64) -> std::cmp::Ordering {
            if number.fract() == 0.0
                && let Some(i) = BigInt::from_f64(number) {
                    return integer.cmp(&i);
                }
            // Non-integer or out-of-range float: compare via f64. If precision
            // makes them appear equal, decide by the sign of the fractional part.
            let ord = integer.to_f64().and_then(|f| f.partial_cmp(&number)).unwrap_or(std::cmp::Ordering::Equal);
            if ord == std::cmp::Ordering::Equal {
                if number.fract() > 0.0 { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }
            } else {
                ord
            }
        }

        let ord = match (left, right) {
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            (Value::Number(a), Value::Number(b)) => {
                if let Some(ord) = a.partial_cmp(b) { ord } else {
                    return Err(Control::RuntimeError("Cannot compare these values".to_string(), span.clone()));
                }
            }
            (Value::Integer(a), Value::Number(b)) => {
                if !b.is_finite() {
                    return Err(Control::RuntimeError("Cannot compare these values".to_string(), span.clone()));
                }
                cmp_integer_number(a, *b)
            }
            (Value::Number(a), Value::Integer(b)) => {
                if !a.is_finite() {
                    return Err(Control::RuntimeError("Cannot compare these values".to_string(), span.clone()));
                }
                cmp_integer_number(b, *a).reverse()
            }
            (Value::String(a), Value::String(b)) => a.cmp(b),
            _ => return Err(Control::RuntimeError("Cannot compare these values".to_string(), span.clone())),
        };
        let a = match ord {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        };
        Ok(Value::Bool(op(a, 0.0)))
    }

    fn eval_neg(&self, v: Value, span: &Span) -> Result<Value, Control> {
        match v {
            Value::Integer(n) => Ok(Value::Integer(-n)),
            Value::Number(n) => Ok(Value::Number(-n)),
            _ => Err(Control::RuntimeError("Cannot negate this value".to_string(), span.clone())),
        }
    }

    fn as_index(&self, value: &Value, len: usize, span: &Span) -> Result<usize, Control> {
        let n = match value {
            Value::Integer(n) => n.clone(),
            Value::Number(n) if n.fract() == 0.0 => BigInt::from_f64(*n).unwrap_or_else(|| BigInt::from(0)),
            _ => return Err(Control::RuntimeError("Index must be integer".to_string(), span.clone())),
        };
        let i = if n < BigInt::from(0) {
            let neg = (-n).to_usize().ok_or_else(|| Control::RuntimeError("Index out of range".to_string(), span.clone()))?;
            if neg > len { return Err(Control::RuntimeError("Index out of range".to_string(), span.clone())); }
            len - neg
        } else {
            n.to_usize().ok_or_else(|| Control::RuntimeError("Index out of range".to_string(), span.clone()))?
        };
        if i >= len { Err(Control::RuntimeError("Index out of range".to_string(), span.clone())) }
        else { Ok(i) }
    }

    fn call_value(&mut self, callee: &Value, args: Vec<Value>, span: &Span) -> Result<Value, Control> {
        match callee {
            Value::BuiltIn { min_arity, max_arity, func, .. } => {
                if args.len() < *min_arity || args.len() > *max_arity {
                    return Err(Control::RuntimeError("Wrong arity".to_string(), span.clone()));
                }
                func(&args).map_err(|m| Control::RuntimeError(m, span.clone()))
            }
            Value::Function { name, params, return_type, body, closure, span: func_span, from_module } => {
                if params.len() != args.len() {
                    return Err(Control::RuntimeError(format!("Function {} expects {} args, got {}", name, params.len(), args.len()), func_span.clone()));
                }
                let env = Environment::with_parent(closure.clone());
                for ((p, ann), a) in params.iter().zip(args) {
                    if let Some(ann) = ann {
                        self.check_type(&a, ann, func_span)?;
                    }
                    env.borrow().define(p, a, ann.clone());
                }
                let old = self.env.clone();
                self.env = env;
                let result = Value::Nothing;
                for stmt in body {
                    if let Err(ctrl) = self.execute(stmt) {
                        self.env = old;
                        return match ctrl {
                            Control::Return(v, span) => {
                                if let Some(ann) = return_type {
                                    self.check_type(&v, ann, &span)?;
                                }
                                Ok(v)
                            }
                            Control::RuntimeError(msg, _) if *from_module => {
                                Err(Control::RuntimeError(msg, span.clone()))
                            }
                            e => Err(e),
                        };
                    }
                }
                self.env = old;
                if let Some(ann) = return_type {
                    self.check_type(&result, ann, func_span)?;
                }
                Ok(result)
            }
            Value::Class { .. } => self.new_instance(callee, args, span),
            _ => Err(Control::RuntimeError(format!("Cannot call {}", callee.type_name()), span.clone())),
        }
    }

    fn new_instance(&mut self, cls: &Value, args: Vec<Value>, span: &Span) -> Result<Value, Control> {
        if let Value::Class { init, methods: _, from_module, .. } = cls {
            let instance = Value::Instance {
                class: Box::new(cls.clone()),
                fields: Rc::new(RefCell::new(HashMap::new())),
            };
            if let Some(init_stmt) = init {
                let env = Environment::with_parent(self.env.clone());
                env.borrow().define_untyped("this", instance.clone());
                for ((p, ann), a) in init_stmt.params.iter().zip(args) {
                    if let Some(ann) = ann {
                        self.check_type(&a, ann, &Span { line: 0, col: 0 })?;
                    }
                    env.borrow().define(p, a, ann.clone());
                }
                let old = self.env.clone();
                self.env = env;
                for stmt in &init_stmt.body {
                    if let Err(ctrl) = self.execute(stmt) {
                        self.env = old;
                        return match ctrl {
                            Control::Return(_, _) => Ok(instance),
                            Control::RuntimeError(msg, _) if *from_module => {
                                Err(Control::RuntimeError(msg, span.clone()))
                            }
                            e => Err(e),
                        };
                    }
                }
                self.env = old;
            }
            Ok(instance)
        } else {
            Err(Control::RuntimeError(format!("Cannot create instance of {}", cls.type_name()), span.clone()))
        }
    }

    fn call_method(&mut self, obj: &Value, method: &str, args: Vec<Value>, span: &Span) -> Result<Value, Control> {
        if let Value::Instance { class, .. } = obj
            && let Value::Class { methods, from_module, .. } = class.as_ref()
                && let Some(Stmt::Define { params, return_type, body, span, .. }) = methods.get(method) {
                    let method_span = span.clone();
                    let env = Environment::with_parent(self.env.clone());
                    env.borrow().define_untyped("this", obj.clone());
                    for ((p, ann), a) in params.iter().zip(args) {
                        if let Some(ann) = ann {
                            self.check_type(&a, ann, &method_span)?;
                        }
                        env.borrow().define(p, a, ann.clone());
                    }
                    let old = self.env.clone();
                    self.env = env;
                    for stmt in body {
                        if let Err(ctrl) = self.execute(stmt) {
                            self.env = old;
                            return match ctrl {
                                Control::Return(v, span) => {
                                    if let Some(ann) = return_type {
                                        self.check_type(&v, ann, &span)?;
                                    }
                                    Ok(v)
                                }
                                Control::RuntimeError(msg, _) if *from_module => {
                                    Err(Control::RuntimeError(msg, span.clone()))
                                }
                                e => Err(e),
                            };
                        }
                    }
                    self.env = old;
                    let result = Value::Nothing;
                    if let Some(ann) = return_type {
                        self.check_type(&result, ann, &method_span)?;
                    }
                    return Ok(result);
                }
        Err(Control::RuntimeError(format!("Cannot send message to {}", obj.type_name()), span.clone()))
    }

    fn iterable_items(&self, value: &Value) -> Result<Vec<Value>, Control> {
        match value {
            Value::List(l) => Ok(l.borrow().clone()),
            Value::String(s) => Ok(s.chars().map(|c| Value::String(c.to_string())).collect()),
            Value::Dict(d) => Ok(d.borrow().keys().map(|k| k.to_value()).collect()),
            _ => Err(Control::Error(format!("Cannot iterate over {}", value.type_name()))),
        }
    }

    pub fn is_truthy(value: &Value) -> bool {
        match value {
            Value::Nothing => false,
            Value::Bool(b) => *b,
            Value::Integer(n) => *n != BigInt::from(0),
            Value::Number(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::List(l) => !l.borrow().is_empty(),
            Value::Dict(d) => !d.borrow().is_empty(),
            _ => true,
        }
    }

    fn import_module(&mut self, path: &str, span: &Span) -> Result<(), Control> {
        if self.modules.borrow().contains_key(path) {
            return Ok(());
        }
        if !self.loading_modules.borrow_mut().insert(path.to_string()) {
            return Err(Control::RuntimeError(format!("Circular import detected: '{}' is already being loaded", path), span.clone()));
        }

        let result = self.import_module_inner(path, span);
        self.loading_modules.borrow_mut().remove(path);
        result
    }

    fn import_module_inner(&mut self, path: &str, span: &Span) -> Result<(), Control> {
        let env = if let Some(file) = find_module_file(path, self.current_path.as_deref()) {
            self.load_period_module(path, &file)?
        } else {
            match path {
                "math" => make_math_module(),
                "random" => make_random_module(),
                "string" => make_string_module(),
                "time" => make_time_module(),
                _ => return Err(Control::RuntimeError(format!("Module '{}' not found", path), span.clone())),
            }
        };

        self.modules.borrow_mut().insert(path.to_string(), env.clone());
        let exposed_name = path.rsplit('/').next().unwrap_or(path);
        self.env.borrow().define_untyped(exposed_name, Value::Module { name: path.to_string(), env: env.clone() });
        let exports = env.borrow().exported_names();
        let filter = !exports.is_empty();
        for (name, value, type_ann) in env.borrow().entries() {
            if !filter || exports.contains(&name) {
                self.env.borrow().define(&name, value, type_ann);
            }
        }
        Ok(())
    }

    fn load_period_module(&mut self, name: &str, path: &std::path::Path) -> Result<Rc<RefCell<Environment>>, Control> {
        let source = fs::read_to_string(path)
            .map_err(|e| Control::Error(format!("Cannot read module '{}': {}", name, e)))?;
        let program = parse_module(&source)
            .map_err(|e| Control::Error(format!("Module '{}': {}", name, e)))?;

        let builtins = Environment::new();
        install_builtins(&builtins.borrow());
        let module_env = Environment::with_parent(builtins);

        let old_env = self.env.clone();
        let old_silent = self.silent;
        let old_loading = self.loading_module;
        self.env = module_env.clone();
        self.silent = true;
        self.loading_module = true;
        let result = self.interpret(&program);
        self.env = old_env;
        self.silent = old_silent;
        self.loading_module = old_loading;

        result?;
        Ok(module_env)
    }
}

fn stdlib_locations() -> Vec<PathBuf> {
    let mut locs = Vec::new();
    if let Ok(v) = env::var("PERIOD_STDLIB") {
        locs.push(PathBuf::from(v));
    }
    if let Ok(exe) = env::current_exe()
        && let Some(parent) = exe.parent() {
            locs.push(parent.join("stdlib"));
            // Development layout: binary next to a `period` project directory.
            locs.push(parent.join("period").join("stdlib"));
            // FHS-style install layout (e.g. /usr/local/bin/period -> /usr/local/share/period/stdlib)
            if let Some(grandparent) = parent.parent() {
                locs.push(grandparent.join("share").join("period").join("stdlib"));
            }
            // Rust cargo development layout: binary is at period/target/<profile>/period,
            // stdlib is at the repository root or under period/stdlib.
            if parent.file_name().map(|n| n == "debug" || n == "release").unwrap_or(false)
                && let Some(repo) = parent.parent().and_then(|p| p.parent()).and_then(|p| p.parent())
            {
                locs.push(repo.join("stdlib"));
                locs.push(repo.join("period").join("stdlib"));
            }
        }
    if let Ok(cwd) = env::current_dir() {
        locs.push(cwd.join("stdlib"));
        // Development layout: run from the repo root while stdlib lives under `period/`.
        locs.push(cwd.join("period").join("stdlib"));
    }
    locs
}

fn find_module_file(module: &str, current_path: Option<&std::path::Path>) -> Option<PathBuf> {
    module_file_candidates(module, current_path).into_iter().find(|candidate| candidate.is_file())
}

fn module_file_candidates(module: &str, current_path: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if module.starts_with("./") || module.starts_with("../") {
        // Relative POSIX-style paths: ./helper or ../utils/helper.
        if let Some(current) = current_path {
            let dir = if current.is_file() {
                current.parent().unwrap_or(current)
            } else {
                current
            };
            let local_path = dir.join(module);
            candidates.push(local_path.with_extension("period"));
            candidates.push(dir.join("lib").join(module).with_extension("period"));
        }
    } else {
        // Plain module names resolve to installed packages, the standard library,
        // or built-in modules. If a lockfile exists, prefer its listed packages.
        let project_root = current_path.and_then(|p| p.parent()).map(PathBuf::from)
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        if let Some(path) = crate::package_manager::package_path_in(module, &project_root) {
            candidates.push(project_root.join(path));
        }
        let file = format!("{}.period", module);
        if let Ok(cwd) = env::current_dir() {
            candidates.push(cwd.join("period_packages").join(&file));
        }
        for loc in stdlib_locations() {
            candidates.push(loc.join(&file));
        }
    }

    candidates
}

fn parse_module(source: &str) -> Result<Program, String> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token()?;
        let eof = matches!(t.kind, TokenKind::Eof);
        tokens.push(t);
        if eof { break; }
    }
    Parser::new(tokens).parse_program()
}

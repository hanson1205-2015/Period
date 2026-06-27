use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::*;
use crate::lexer::{Lexer, TokenKind};
use crate::parser::Parser;

#[derive(Clone)]
pub enum Value {
    Integer(i64),
    Number(f64),
    String(String),
    Bool(bool),
    Nothing,
    List(Rc<RefCell<Vec<Value>>>),
    Dict(Rc<RefCell<HashMap<ValueKey, Value>>>),
    Function {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
        closure: Rc<RefCell<Environment>>,
    },
    Class {
        name: String,
        init: Option<Init>,
        methods: HashMap<String, Stmt>,
    },
    Instance {
        class: Box<Value>,
        fields: Rc<RefCell<HashMap<String, Value>>>,
    },
    BuiltIn {
        name: String,
        min_arity: usize,
        max_arity: usize,
        func: fn(&[Value]) -> Result<Value, String>,
    },
    Module {
        name: String,
        env: Rc<RefCell<Environment>>,
    },
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Integer(n) => write!(f, "{}", n),
            Value::Number(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            Value::Nothing => write!(f, "nothing"),
            Value::List(l) => write!(f, "{:?}", l.borrow()),
            Value::Dict(d) => write!(f, "{:?}", d.borrow()),
            Value::Function { name, .. } => write!(f, "<function {}>", name),
            Value::Class { name, .. } => write!(f, "<class {}>", name),
            Value::Instance { class, .. } => write!(f, "<instance of {:?}>", class),
            Value::BuiltIn { name, .. } => write!(f, "<built-in {}>", name),
            Value::Module { name, .. } => write!(f, "<module {}>", name),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Nothing, Value::Nothing) => true,
            (Value::List(a), Value::List(b)) => a.borrow().eq(&*b.borrow()),
            (Value::Dict(a), Value::Dict(b)) => a.borrow().eq(&*b.borrow()),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueKey {
    Integer(i64),
    String(String),
    Bool(bool),
    Nothing,
}

impl Value {
    fn as_key(&self) -> Result<ValueKey, String> {
        match self {
            Value::Integer(n) => Ok(ValueKey::Integer(*n)),
            Value::String(s) => Ok(ValueKey::String(s.clone())),
            Value::Bool(b) => Ok(ValueKey::Bool(*b)),
            Value::Nothing => Ok(ValueKey::Nothing),
            _ => Err(format!("{} is not hashable", self.type_name())),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Integer(_) => "integer",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Bool(_) => "boolean",
            Value::Nothing => "nothing",
            Value::List(_) => "list",
            Value::Dict(_) => "dictionary",
            Value::Function { .. } => "function",
            Value::Class { .. } => "class",
            Value::Instance { .. } => "instance",
            Value::BuiltIn { .. } => "built-in",
            Value::Module { .. } => "module",
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            Value::Integer(n) => n.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            Value::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
            Value::Nothing => "nothing".to_string(),
            Value::List(l) => {
                let items: Vec<String> = l.borrow().iter().map(|v| v.to_string()).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Dict(d) => {
                let items: Vec<String> = d.borrow().iter()
                    .map(|(k, v)| format!("{}: {}", k.to_value().to_string(), v.to_string()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            Value::Function { name, .. } => format!("<function {}>", name),
            Value::Class { name, .. } => format!("<class {}>", name),
            Value::Instance { class, .. } => format!("<instance of {:?}>", class),
            Value::BuiltIn { name, .. } => format!("<built-in {}>", name),
            Value::Module { name, .. } => format!("<module {}>", name),
        }
    }
}

impl ValueKey {
    fn to_value(&self) -> Value {
        match self {
            ValueKey::Integer(n) => Value::Integer(*n),
            ValueKey::String(s) => Value::String(s.clone()),
            ValueKey::Bool(b) => Value::Bool(*b),
            ValueKey::Nothing => Value::Nothing,
        }
    }
    fn to_string(&self) -> String { self.to_value().to_string() }
}

#[derive(Clone)]
pub struct Environment {
    values: RefCell<HashMap<String, Value>>,
    parent: Option<Rc<RefCell<Environment>>>,
}

impl Environment {
    pub fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self { values: RefCell::new(HashMap::new()), parent: None }))
    }

    pub fn with_parent(parent: Rc<RefCell<Self>>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self { values: RefCell::new(HashMap::new()), parent: Some(parent) }))
    }

    pub fn define(&self, name: &str, value: Value) {
        self.values.borrow_mut().insert(name.to_string(), value);
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.values.borrow().get(name).cloned() {
            return Some(v);
        }
        if let Some(parent) = &self.parent {
            return parent.borrow().get(name);
        }
        None
    }

    pub fn set(&self, name: &str, value: Value) -> Result<(), String> {
        if self.values.borrow().contains_key(name) {
            self.values.borrow_mut().insert(name.to_string(), value);
            return Ok(());
        }
        if let Some(parent) = &self.parent {
            return parent.borrow().set(name, value);
        }
        Err(format!("Undefined variable '{}'", name))
    }
}

#[derive(Debug)]
pub enum Control {
    Return(Value),
    Error(String),
}

pub struct Interpreter {
    globals: Rc<RefCell<Environment>>,
    env: Rc<RefCell<Environment>>,
    pub output: Vec<String>,
    modules: RefCell<HashMap<String, Rc<RefCell<Environment>>>>,
    silent: bool,
    current_path: Option<PathBuf>,
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = Environment::new();
        install_builtins(&*globals.borrow());
        let env = globals.clone();
        Self { globals, env, output: Vec::new(), modules: RefCell::new(HashMap::new()), silent: false, current_path: None }
    }

    pub fn set_current_path(&mut self, path: impl Into<PathBuf>) {
        self.current_path = Some(path.into());
    }

    pub fn interpret(&mut self, program: &Program) -> Result<(), Control> {
        for stmt in &program.statements {
            self.execute(stmt)?;
        }
        Ok(())
    }

    fn execute(&mut self, stmt: &Stmt) -> Result<(), Control> {
        match stmt {
            Stmt::Let { name, value } => {
                let v = self.evaluate(value)?;
                self.env.borrow().define(name, v);
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
                let items = self.iterable_items(&iter_value)?;
                for item in items {
                    let env = Environment::with_parent(self.env.clone());
                    env.borrow().define(var, item);
                    let old = self.env.clone();
                    self.env = env;
                    let result = self.execute_block(body);
                    self.env = old;
                    result?;
                }
            }
            Stmt::Return(value) => {
                let v = match value { Some(e) => self.evaluate(e)?, None => Value::Nothing };
                return Err(Control::Return(v));
            }
            Stmt::Define { name, params, body, .. } => {
                let func = Value::Function {
                    name: name.clone(),
                    params: params.iter().map(|(n, _)| n.clone()).collect(),
                    body: body.clone(),
                    closure: self.env.clone(),
                };
                self.env.borrow().define(name, func);
            }
            Stmt::Init(_) => {}
            Stmt::Class { name, init, methods, .. } => {
                let mut method_map = HashMap::new();
                for m in methods {
                    if let Stmt::Define { name: mname, params, body, .. } = m {
                        method_map.insert(mname.clone(), Stmt::Define { name: mname.clone(), params: params.clone(), return_type: None, docstring: None, body: body.clone() });
                    }
                }
                self.env.borrow().define(name, Value::Class { name: name.clone(), init: init.clone(), methods: method_map });
            }
            Stmt::Import(paths) => {
                for path in paths { self.import_module(path)?; }
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
            AssignTarget::Variable(name) => {
                self.env.borrow().set(name, value).map_err(Control::Error)?;
            }
            AssignTarget::Index { object, index } => {
                let obj = self.evaluate(object)?;
                let idx = self.evaluate(index)?;
                match obj {
                    Value::List(list) => {
                        let i = self.as_index(&idx, list.borrow().len())?;
                        list.borrow_mut()[i] = value;
                    }
                    Value::Dict(dict) => {
                        let key = idx.as_key().map_err(Control::Error)?;
                        dict.borrow_mut().insert(key, value);
                    }
                    _ => return Err(Control::Error(format!("Cannot index into {}", obj.type_name()))),
                }
            }
            AssignTarget::Property { object, name } => {
                let obj = self.evaluate(object)?;
                if let Value::Instance { fields, .. } = obj {
                    fields.borrow_mut().insert(name.clone(), value);
                } else {
                    return Err(Control::Error(format!("Cannot set property on {}", obj.type_name())));
                }
            }
        }
        Ok(())
    }

    fn evaluate(&mut self, expr: &Expr) -> Result<Value, Control> {
        match expr {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::String(s) => Ok(Value::String(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nothing => Ok(Value::Nothing),
            Expr::Variable { name, .. } => {
                let value = self.env.borrow().get(name).ok_or_else(|| Control::Error(format!("Undefined variable '{}'", name)))?;
                // Zero-argument functions (like input or random) are called automatically when used as a value.
                if let Value::BuiltIn { min_arity: 0, max_arity: 0, .. } = &value {
                    return self.call_value(&value, vec![]);
                }
                if let Value::Function { params, .. } = &value {
                    if params.is_empty() {
                        return self.call_value(&value, vec![]);
                    }
                }
                Ok(value)
            }
            Expr::Binary { op, left, right } => {
                let l = self.evaluate(left)?;
                // short-circuit
                match op {
                    BinOp::And => { if !Self::is_truthy(&l) { return Ok(l); } return Ok(self.evaluate(right)?); }
                    BinOp::Or => { if Self::is_truthy(&l) { return Ok(l); } return Ok(self.evaluate(right)?); }
                    _ => {}
                }
                let r = self.evaluate(right)?;
                self.eval_binary(op, l, r)
            }
            Expr::Unary { op, operand } => {
                let v = self.evaluate(operand)?;
                match op {
                    UnaryOp::Neg => self.eval_neg(v),
                    UnaryOp::Not => Ok(Value::Bool(!Self::is_truthy(&v))),
                }
            }
            Expr::Call { callee, args } => {
                let c = self.evaluate(callee)?;
                let a: Result<Vec<_>, _> = args.iter().map(|e| self.evaluate(e)).collect();
                self.call_value(&c, a?)
            }
            Expr::Index { object, index } => {
                let obj = self.evaluate(object)?;
                let idx = self.evaluate(index)?;
                match obj {
                    Value::List(list) => {
                        let i = self.as_index(&idx, list.borrow().len())?;
                        Ok(list.borrow()[i].clone())
                    }
                    Value::Dict(dict) => {
                        let key = idx.as_key().map_err(Control::Error)?;
                        dict.borrow().get(&key).cloned().ok_or_else(|| Control::Error(format!("Key not found")))
                    }
                    Value::String(s) => {
                        let i = self.as_index(&idx, s.len())?;
                        Ok(Value::String(s.chars().nth(i).unwrap().to_string()))
                    }
                    _ => Err(Control::Error(format!("Cannot index into {}", obj.type_name()))),
                }
            }
            Expr::Property { object, name } => {
                let obj = self.evaluate(object)?;
                match &obj {
                    Value::Instance { class, fields } => {
                        if let Some(v) = fields.borrow().get(name).cloned() {
                            return Ok(v);
                        }
                        if let Value::Class { methods, .. } = class.as_ref() {
                            if let Some(Stmt::Define { params, body, .. }) = methods.get(name) {
                                return Ok(Value::Function { name: name.clone(), params: params.iter().map(|(n,_)| n.clone()).collect(), body: body.clone(), closure: self.env.clone() });
                            }
                        }
                        Err(Control::Error(format!("Instance has no property '{}'", name)))
                    }
                    Value::Module { env, .. } => {
                        env.borrow().get(name).ok_or_else(|| Control::Error(format!("Module has no property '{}'", name)))
                    }
                    _ => Err(Control::Error(format!("Cannot access property on {}", obj.type_name()))),
                }
            }
            Expr::New { class, .. } => {
                self.evaluate(class)
            }
            Expr::Tell { object, method, args } => {
                let obj = self.evaluate(object)?;
                let a: Result<Vec<_>, _> = args.iter().map(|e| self.evaluate(e)).collect();
                self.call_method(&obj, method, a?)
            }
            Expr::Qualified { name, module } => {
                let mod_env = self.modules.borrow().get(module).cloned()
                    .ok_or_else(|| Control::Error(format!("Module '{}' not imported", module)))?;
                mod_env.borrow().get(name).ok_or_else(|| Control::Error(format!("'{}' not found in module '{}'", name, module)))
            }
            Expr::List(elems) => {
                let mut vals = Vec::new();
                for e in elems { vals.push(self.evaluate(e)?); }
                Ok(Value::List(Rc::new(RefCell::new(vals))))
            }
            Expr::Dict(pairs) => {
                let mut map = HashMap::new();
                for (k, v) in pairs {
                    let key = self.evaluate(k)?.as_key().map_err(Control::Error)?;
                    map.insert(key, self.evaluate(v)?);
                }
                Ok(Value::Dict(Rc::new(RefCell::new(map))))
            }
        }
    }

    fn eval_binary(&self, op: &BinOp, left: Value, right: Value) -> Result<Value, Control> {
        match op {
            BinOp::Add => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a + b)),
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
                (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
                (Value::List(a), Value::List(b)) => {
                    let mut items = a.borrow().clone();
                    items.extend(b.borrow().iter().cloned());
                    Ok(Value::List(Rc::new(RefCell::new(items))))
                }
                _ => Err(Control::Error("Invalid operands for +".to_string())),
            },
            BinOp::Sub => self.numeric_op(&left, &right, |a,b| a - b, |a,b| (a - b) as f64),
            BinOp::Mul => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a * b)),
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
                (Value::String(s), Value::Integer(n)) | (Value::Integer(n), Value::String(s)) => {
                    Ok(Value::String(s.repeat(*n as usize)))
                }
                _ => Err(Control::Error("Invalid operands for *".to_string())),
            },
            BinOp::Div => self.numeric_op(&left, &right, |a,b| if b == 0.0 { f64::NAN } else { a / b }, |a,b| if b == 0 { f64::NAN } else { a as f64 / b as f64 }),
            BinOp::Mod => self.numeric_op(&left, &right, |a,b| a % b, |a,b| (a % b) as f64),
            BinOp::Pow => self.numeric_op(&left, &right, |a,b| a.powf(b), |a,b| a.pow(b as u32) as f64),
            BinOp::Eq => Ok(Value::Bool(left == right)),
            BinOp::Ne => Ok(Value::Bool(left != right)),
            BinOp::Lt => self.compare(&left, &right, |a,b| a < b),
            BinOp::Gt => self.compare(&left, &right, |a,b| a > b),
            BinOp::Le => self.compare(&left, &right, |a,b| a <= b),
            BinOp::Ge => self.compare(&left, &right, |a,b| a >= b),
            _ => unreachable!(),
        }
    }

    fn numeric_op<FN, FI>(&self, left: &Value, right: &Value, float_op: FN, int_to_float: FI) -> Result<Value, Control>
    where FN: Fn(f64, f64) -> f64, FI: Fn(i64, i64) -> f64 {
        match (left, right) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Number(int_to_float(*a, *b))),
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(float_op(*a, *b))),
            (Value::Integer(a), Value::Number(b)) => Ok(Value::Number(float_op(*a as f64, *b))),
            (Value::Number(a), Value::Integer(b)) => Ok(Value::Number(float_op(*a, *b as f64))),
            _ => Err(Control::Error("Operands must be numbers".to_string())),
        }
    }

    fn compare<F>(&self, left: &Value, right: &Value, op: F) -> Result<Value, Control>
    where F: Fn(f64, f64) -> bool {
        match (left, right) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Bool(op(*a as f64, *b as f64))),
            (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(op(*a, *b))),
            (Value::Integer(a), Value::Number(b)) => Ok(Value::Bool(op(*a as f64, *b))),
            (Value::Number(a), Value::Integer(b)) => Ok(Value::Bool(op(*a, *b as f64))),
            (Value::String(a), Value::String(b)) => {
                let v = match a.cmp(b) { std::cmp::Ordering::Less => -1.0, std::cmp::Ordering::Equal => 0.0, std::cmp::Ordering::Greater => 1.0 };
                Ok(Value::Bool(op(v, 0.0)))
            }
            _ => Err(Control::Error("Cannot compare these values".to_string())),
        }
    }

    fn eval_neg(&self, v: Value) -> Result<Value, Control> {
        match v {
            Value::Integer(n) => Ok(Value::Integer(-n)),
            Value::Number(n) => Ok(Value::Number(-n)),
            _ => Err(Control::Error("Cannot negate this value".to_string())),
        }
    }

    fn as_index(&self, value: &Value, len: usize) -> Result<usize, Control> {
        let n = match value {
            Value::Integer(n) => *n,
            Value::Number(n) if n.fract() == 0.0 => *n as i64,
            _ => return Err(Control::Error("Index must be integer".to_string())),
        };
        let i = if n < 0 { len as i64 + n } else { n } as usize;
        if i >= len { Err(Control::Error("Index out of range".to_string())) }
        else { Ok(i) }
    }

    fn call_value(&mut self, callee: &Value, args: Vec<Value>) -> Result<Value, Control> {
        match callee {
            Value::BuiltIn { min_arity, max_arity, func, .. } => {
                if args.len() < *min_arity || args.len() > *max_arity {
                    return Err(Control::Error(format!("Wrong arity")));
                }
                func(&args).map_err(Control::Error)
            }
            Value::Function { name, params, body, closure } => {
                if params.len() != args.len() {
                    return Err(Control::Error(format!("Function {} expects {} args, got {}", name, params.len(), args.len())));
                }
                let env = Environment::with_parent(closure.clone());
                for (p, a) in params.iter().zip(args) { env.borrow().define(p, a); }
                let old = self.env.clone();
                self.env = env;
                let mut result = Value::Nothing;
                for stmt in body {
                    if let Err(ctrl) = self.execute(stmt) {
                        self.env = old;
                        return match ctrl { Control::Return(v) => Ok(v), e => Err(e) };
                    }
                }
                self.env = old;
                Ok(result)
            }
            Value::Class { .. } => self.new_instance(callee, args),
            _ => Err(Control::Error(format!("Cannot call {}", callee.type_name()))),
        }
    }

    fn new_instance(&mut self, cls: &Value, args: Vec<Value>) -> Result<Value, Control> {
        if let Value::Class { name, init, methods } = cls {
            let instance = Value::Instance {
                class: Box::new(cls.clone()),
                fields: Rc::new(RefCell::new(HashMap::new())),
            };
            if let Some(init_stmt) = init {
                let env = Environment::with_parent(self.env.clone());
                env.borrow().define("this", instance.clone());
                for ((p, _), a) in init_stmt.params.iter().zip(args) { env.borrow().define(p, a); }
                let old = self.env.clone();
                self.env = env;
                for stmt in &init_stmt.body {
                    if let Err(ctrl) = self.execute(stmt) {
                        self.env = old;
                        return match ctrl { Control::Return(_) => Ok(instance), e => Err(e) };
                    }
                }
                self.env = old;
            }
            Ok(instance)
        } else {
            Err(Control::Error(format!("Cannot create instance of {}", cls.type_name())))
        }
    }

    fn call_method(&mut self, obj: &Value, method: &str, args: Vec<Value>) -> Result<Value, Control> {
        if let Value::Instance { class, .. } = obj {
            if let Value::Class { methods, .. } = class.as_ref() {
                if let Some(Stmt::Define { params, body, .. }) = methods.get(method) {
                    let env = Environment::with_parent(self.env.clone());
                    env.borrow().define("this", obj.clone());
                    for ((p, _), a) in params.iter().zip(args) { env.borrow().define(p, a); }
                    let old = self.env.clone();
                    self.env = env;
                    for stmt in body {
                        if let Err(ctrl) = self.execute(stmt) {
                            self.env = old;
                            return match ctrl { Control::Return(v) => Ok(v), e => Err(e) };
                        }
                    }
                    self.env = old;
                    return Ok(Value::Nothing);
                }
            }
        }
        Err(Control::Error(format!("Cannot send message to {}", obj.type_name())))
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
            Value::Integer(n) => *n != 0,
            Value::Number(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::List(l) => !l.borrow().is_empty(),
            Value::Dict(d) => !d.borrow().is_empty(),
            _ => true,
        }
    }

    fn import_module(&mut self, path: &str) -> Result<(), Control> {
        if self.modules.borrow().contains_key(path) {
            return Ok(());
        }

        let env = if let Some(file) = find_module_file(path, self.current_path.as_deref()) {
            self.load_period_module(path, &file)?
        } else {
            match path {
                "math" => make_math_module(),
                "random" => make_random_module(),
                "string" => make_string_module(),
                "time" => make_time_module(),
                _ => return Err(Control::Error(format!("Module '{}' not found", path))),
            }
        };

        self.modules.borrow_mut().insert(path.to_string(), env.clone());
        let exposed_name = path.rsplit('.').next().unwrap_or(path);
        self.env.borrow().define(exposed_name, Value::Module { name: path.to_string(), env: env.clone() });
        for (name, value) in env.borrow().values.borrow().iter() {
            self.env.borrow().define(name, value.clone());
        }
        Ok(())
    }

    fn load_period_module(&mut self, name: &str, path: &std::path::Path) -> Result<Rc<RefCell<Environment>>, Control> {
        let source = fs::read_to_string(path)
            .map_err(|e| Control::Error(format!("Cannot read module '{}': {}", name, e)))?;
        let program = parse_module(&source)
            .map_err(|e| Control::Error(format!("Module '{}': {}", name, e)))?;

        let builtins = Environment::new();
        install_builtins(&*builtins.borrow());
        let module_env = Environment::with_parent(builtins);

        let old_env = self.env.clone();
        let old_silent = self.silent;
        self.env = module_env.clone();
        self.silent = true;
        let result = self.interpret(&program);
        self.env = old_env;
        self.silent = old_silent;

        result?;
        Ok(module_env)
    }
}

fn stdlib_locations() -> Vec<PathBuf> {
    let mut locs = Vec::new();
    if let Ok(v) = env::var("PERIOD_STDLIB") {
        locs.push(PathBuf::from(v));
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            locs.push(parent.join("stdlib"));
        }
    }
    if let Ok(cwd) = env::current_dir() {
        locs.push(cwd.join("stdlib"));
    }
    locs
}

fn find_module_file(module: &str, current_path: Option<&std::path::Path>) -> Option<PathBuf> {
    for candidate in module_file_candidates(module, current_path) {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn module_file_candidates(module: &str, current_path: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if module.starts_with('.') {
        // Relative local modules: .abc (current directory) or ..abc (parent directory).
        if let Some(current) = current_path {
            let dir = if current.is_file() {
                current.parent().unwrap_or(current)
            } else {
                current
            };

            let (relative_depth, parts) = parse_relative_module(module);
            let mut base = dir.to_path_buf();
            for _ in 0..relative_depth {
                base = base.join("..");
            }
            let local_path = if parts.is_empty() {
                base.clone()
            } else {
                base.join(parts.join(std::path::MAIN_SEPARATOR_STR))
            };
            candidates.push(local_path.with_extension("period"));
            candidates.push(base.join("lib").join(parts.join(std::path::MAIN_SEPARATOR_STR)).with_extension("period"));
        }
    } else {
        // Plain module names resolve to the standard library or built-in modules only.
        let file = format!("{}.period", module);
        for loc in stdlib_locations() {
            candidates.push(loc.join(&file));
        }
    }

    candidates
}

fn parse_relative_module(module: &str) -> (usize, Vec<&str>) {
    let dots = module.chars().take_while(|&c| c == '.').count();
    let rest = &module[dots..];
    let parts: Vec<&str> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split('.').collect()
    };
    // ".helper"  -> depth 0 (current dir), "..helper" -> depth 1 (one level up), etc.
    (dots.saturating_sub(1), parts)
}

fn parse_module(source: &str) -> Result<Program, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let t = lexer.next_token();
            let eof = matches!(t.kind, TokenKind::Eof);
            tokens.push(t);
            if eof { break; }
        }
        Parser::new(tokens).parse_program()
    }))
    .map_err(|_| "parse error".to_string())
}

fn install_builtins(env: &Environment) {
    env.define("length", Value::BuiltIn { name: "length".to_string(), min_arity: 1, max_arity: 1, func: builtin_length });
    env.define("string", Value::BuiltIn { name: "string".to_string(), min_arity: 1, max_arity: 1, func: builtin_string });
    env.define("number", Value::BuiltIn { name: "number".to_string(), min_arity: 1, max_arity: 1, func: builtin_number });
    env.define("type", Value::BuiltIn { name: "type".to_string(), min_arity: 1, max_arity: 1, func: builtin_type });
    env.define("input", Value::BuiltIn { name: "input".to_string(), min_arity: 0, max_arity: 0, func: builtin_input });
    env.define("range", Value::BuiltIn { name: "range".to_string(), min_arity: 1, max_arity: 3, func: builtin_range });
}

fn builtin_length(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::String(s) => Ok(Value::Integer(s.len() as i64)),
        Value::List(l) => Ok(Value::Integer(l.borrow().len() as i64)),
        Value::Dict(d) => Ok(Value::Integer(d.borrow().len() as i64)),
        _ => Err("Cannot get length".to_string()),
    }
}

fn builtin_string(args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(args[0].to_string()))
}

fn builtin_number(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Integer(n) => Ok(Value::Number(*n as f64)),
        Value::Number(n) => Ok(Value::Number(*n)),
        Value::String(s) => s.parse::<f64>().map(Value::Number).or_else(|_| s.parse::<i64>().map(Value::Integer)).map_err(|_| "Cannot convert to number".to_string()),
        Value::Bool(true) => Ok(Value::Number(1.0)),
        Value::Bool(false) => Ok(Value::Number(0.0)),
        _ => Err("Cannot convert to number".to_string()),
    }
}

fn builtin_type(args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(args[0].type_name().to_string()))
}

fn builtin_input(_: &[Value]) -> Result<Value, String> {
    let mut s = String::new();
    io::stdout().flush().unwrap();
    io::stdin().read_line(&mut s).map_err(|e| e.to_string())?;
    Ok(Value::String(s.trim_end().to_string()))
}

fn builtin_range(args: &[Value]) -> Result<Value, String> {
    let to_i = |v: &Value| match v { Value::Integer(n) => Ok(*n), Value::Number(n) => Ok(*n as i64), _ => Err("range args must be integers".to_string()) };
    let (start, stop, step) = match args.len() {
        1 => (0, to_i(&args[0])?, 1),
        2 => (to_i(&args[0])?, to_i(&args[1])?, 1),
        3 => (to_i(&args[0])?, to_i(&args[1])?, to_i(&args[2])?),
        _ => unreachable!(),
    };
    if step == 0 { return Err("range step cannot be zero".to_string()); }
    let mut items = Vec::new();
    let mut i = start;
    while (step > 0 && i < stop) || (step < 0 && i > stop) {
        items.push(Value::Integer(i));
        i += step;
    }
    Ok(Value::List(Rc::new(RefCell::new(items))))
}

fn make_module(values: Vec<(&str, Value)>) -> Rc<RefCell<Environment>> {
    let env = Environment::new();
    for (name, value) in values { env.borrow().define(name, value); }
    env
}

fn make_math_module() -> Rc<RefCell<Environment>> {
    macro_rules! unary_float {
        ($name:ident, $f:path) => {
            fn $name(args: &[Value]) -> Result<Value, String> {
                let n = match &args[0] { Value::Integer(i) => *i as f64, Value::Number(n) => *n, _ => return Err("expected number".to_string()) };
                Ok(Value::Number($f(n)))
            }
        };
    }
    unary_float!(sin_fn, f64::sin);
    unary_float!(cos_fn, f64::cos);
    unary_float!(tan_fn, f64::tan);
    unary_float!(sqrt_fn, f64::sqrt);
    unary_float!(abs_fn, f64::abs);
    unary_float!(floor_fn, f64::floor);
    unary_float!(ceil_fn, f64::ceil);
    make_module(vec![
        ("sin", Value::BuiltIn { name: "sin".to_string(), min_arity: 1, max_arity: 1, func: sin_fn }),
        ("cos", Value::BuiltIn { name: "cos".to_string(), min_arity: 1, max_arity: 1, func: cos_fn }),
        ("tan", Value::BuiltIn { name: "tan".to_string(), min_arity: 1, max_arity: 1, func: tan_fn }),
        ("sqrt", Value::BuiltIn { name: "sqrt".to_string(), min_arity: 1, max_arity: 1, func: sqrt_fn }),
        ("abs", Value::BuiltIn { name: "abs".to_string(), min_arity: 1, max_arity: 1, func: abs_fn }),
        ("floor", Value::BuiltIn { name: "floor".to_string(), min_arity: 1, max_arity: 1, func: floor_fn }),
        ("ceil", Value::BuiltIn { name: "ceil".to_string(), min_arity: 1, max_arity: 1, func: ceil_fn }),
        ("pi", Value::Number(std::f64::consts::PI)),
    ])
}

fn make_random_module() -> Rc<RefCell<Environment>> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEED: AtomicU64 = AtomicU64::new(0);
    fn random_fn(_: &[Value]) -> Result<Value, String> {
        let mut seed = SEED.load(Ordering::Relaxed);
        if seed == 0 {
            seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
        }
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        SEED.store(seed, Ordering::Relaxed);
        let r = ((seed >> 33) as f64) / ((1u64 << 31) as f64);
        Ok(Value::Number(r))
    }
    make_module(vec![
        ("random", Value::BuiltIn { name: "random".to_string(), min_arity: 0, max_arity: 0, func: random_fn }),
    ])
}

fn make_string_module() -> Rc<RefCell<Environment>> {
    make_module(vec![
        ("upper", Value::BuiltIn { name: "upper".to_string(), min_arity: 1, max_arity: 1, func: |args| {
            match &args[0] { Value::String(s) => Ok(Value::String(s.to_uppercase())), _ => Err("expected string".to_string()) }
        }}),
        ("lower", Value::BuiltIn { name: "lower".to_string(), min_arity: 1, max_arity: 1, func: |args| {
            match &args[0] { Value::String(s) => Ok(Value::String(s.to_lowercase())), _ => Err("expected string".to_string()) }
        }}),
    ])
}

fn make_time_module() -> Rc<RefCell<Environment>> {
    make_module(vec![
        ("now", Value::BuiltIn { name: "now".to_string(), min_arity: 0, max_arity: 0, func: |_| {
            Ok(Value::Number(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()))
        }}),
    ])
}

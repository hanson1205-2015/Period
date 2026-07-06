//! C-ABI runtime helpers used by the generic Cranelift JIT.
//!
//! The generic JIT represents every Period value as an opaque `*mut Value`.
//! Native code drives control flow; these helpers implement the actual
//! semantics for arithmetic, comparisons, truthiness, composite values,
//! globals, calls, I/O, and more.

use std::cell::RefCell;
use std::rc::Rc;

use num_traits::{FromPrimitive, ToPrimitive};

use crate::ast::{BinOp, UnaryOp};
use crate::bytecode::CompiledFunction;
use crate::interpreter::{Interpreter, Control};
use crate::value::{ErrorValue, Integer, Value, VMClassValue, VMFunctionValue};

fn cstr(ptr: *const u8, len: usize) -> String {
    unsafe {
        if ptr.is_null() {
            return String::new();
        }
        let bytes = std::slice::from_raw_parts(ptr, len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn argv_to_vec(argc: usize, argv: *const *mut Value) -> Vec<Value> {
    unsafe {
        let mut out = Vec::with_capacity(argc);
        if !argv.is_null() {
            let slice = std::slice::from_raw_parts(argv, argc);
            for &p in slice {
                out.push(if p.is_null() { Value::Nothing } else { (*p).clone() });
            }
        }
        out
    }
}

fn take_value(v: *mut Value) -> Value {
    unsafe {
        if v.is_null() {
            return Value::Nothing;
        }
        let value = Box::from_raw(v);
        *value
    }
}

thread_local! {
    static CURRENT_SPAN: std::cell::Cell<(i64, i64)> = std::cell::Cell::new((0, 0));
}

/// Set the current source span for any runtime error raised by the JIT helpers.
#[unsafe(no_mangle)]
pub extern "C" fn period_set_span(line: i64, col: i64) {
    CURRENT_SPAN.with(|s| s.set((line, col)));
}

fn current_span() -> (i64, i64) {
    CURRENT_SPAN.with(|s| s.get())
}

fn make_error(message: impl Into<String>) -> Value {
    let (line, col) = current_span();
    Value::Error(Box::new(ErrorValue {
        message: message.into(),
        line,
        col,
    }))
}

fn error_value(message: impl Into<String>) -> *mut Value {
    Box::into_raw(Box::new(make_error(message)))
}

/// Create an error value from a message value.  Takes ownership of `msg`.
#[unsafe(no_mangle)]
pub extern "C" fn period_raise(msg: *mut Value) -> *mut Value {
    let msg = take_value(msg);
    let message = match msg {
        Value::String(s) => s,
        v => v.to_string(),
    };
    error_value(message)
}

fn result_to_ptr(result: Result<Value, Control>) -> *mut Value {
    match result {
        Ok(v) => Box::into_raw(Box::new(v)),
        Err(Control::RuntimeError(msg, span)) => {
            period_set_span(span.line as i64, span.col as i64);
            error_value(msg)
        }
        Err(Control::Error(msg)) => error_value(msg),
        Err(Control::Return(v, _)) => Box::into_raw(Box::new(v)),
    }
}

/// Clone a value and return a new owned pointer.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_clone(v: *const Value) -> *mut Value {
    unsafe {
        if v.is_null() {
            return std::ptr::null_mut();
        }
        Box::into_raw(Box::new((*v).clone()))
    }
}

/// Drop a value previously returned by a runtime helper.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_drop(v: *mut Value) {
    unsafe {
        if !v.is_null() {
            drop(Box::from_raw(v));
        }
    }
}

/// Return 1 if the value is an error.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_is_error(v: *const Value) -> i64 {
    unsafe {
        match v.as_ref() {
            Some(Value::Error(_)) => 1,
            _ => 0,
        }
    }
}

/// Create a value from an i64.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_from_i64(n: i64) -> *mut Value {
    Box::into_raw(Box::new(Value::Integer(Integer::Small(n))))
}

/// If the value is a small integer, return it; otherwise return 0.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_as_i64(v: *const Value) -> i64 {
    unsafe {
        match v.as_ref() {
            Some(Value::Integer(Integer::Small(n))) => *n,
            _ => 0,
        }
    }
}

/// Create a value from an f64.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_from_f64(n: f64) -> *mut Value {
    Box::into_raw(Box::new(Value::Number(n)))
}

/// Create a boolean value.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_from_bool(b: i64) -> *mut Value {
    Box::into_raw(Box::new(Value::Bool(b != 0)))
}

/// Create the Nothing value.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_nothing() -> *mut Value {
    Box::into_raw(Box::new(Value::Nothing))
}

/// Make a string constant available to the JIT.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_from_string(ptr: *const u8, len: usize) -> *mut Value {
    Box::into_raw(Box::new(Value::String(cstr(ptr, len))))
}

/// Return 1 if the value is truthy, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_truthy(v: *const Value) -> i64 {
    unsafe {
        match v.as_ref() {
            Some(v) => Interpreter::is_truthy(v) as i64,
            None => 0,
        }
    }
}

/// Print a value to stdout / interpreter output.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_show(interp: *mut Interpreter, v: *const Value) {
    unsafe {
        if let Some(v) = v.as_ref() {
            let text = v.to_string();
            if let Some(interp) = interp.as_mut() {
                interp.output.push(text.clone());
                if !interp.silent {
                    println!("{}", text);
                }
            } else {
                println!("{}", text);
            }
        }
    }
}

/// Binary operations on Values.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_binary(op: BinOp, left: *const Value, right: *const Value) -> *mut Value {
    unsafe {
        let left = match left.as_ref() {
            Some(v) => v,
            None => return error_value("missing left operand"),
        };
        let right = match right.as_ref() {
            Some(v) => v,
            None => return error_value("missing right operand"),
        };

        let result = match op {
            BinOp::Add => add_values(left, right),
            BinOp::Sub => sub_values(left, right),
            BinOp::Mul => mul_values(left, right),
            BinOp::Div => div_values(left, right),
            BinOp::Mod => mod_values(left, right),
            BinOp::Pow => pow_values(left, right),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => cmp_values(op, left, right),
            BinOp::And | BinOp::Or => make_error("unexpected binary and/or in JIT"),
        };

        Box::into_raw(Box::new(result))
    }
}

fn add_values(left: &Value, right: &Value) -> Value {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => Value::Integer(a.add(b)),
        (Value::Integer(a), Value::Number(b)) => Value::Number(a.to_f64() + b),
        (Value::Number(a), Value::Integer(b)) => Value::Number(a + b.to_f64()),
        (Value::Number(a), Value::Number(b)) => Value::Number(a + b),
        (Value::String(a), Value::String(b)) => {
            let mut s = String::with_capacity(a.len() + b.len());
            s.push_str(a);
            s.push_str(b);
            Value::String(s)
        }
        (Value::List(a), Value::List(b)) => {
            let ab = a.borrow();
            let bb = b.borrow();
            let mut items = Vec::with_capacity(ab.len() + bb.len());
            items.extend(ab.iter().cloned());
            items.extend(bb.iter().cloned());
            Value::List(std::rc::Rc::new(std::cell::RefCell::new(items)))
        }
        _ => make_error(format!("cannot add {} and {}", left.type_name(), right.type_name())),
    }
}

/// Append a literal string to a local in place when it already holds a string.
/// Otherwise fall back to normal string concatenation and assign the result.
/// Returns null on success so the caller does not need to drop a boxed Nothing.
#[unsafe(no_mangle)]
pub extern "C" fn period_append_local_string(local: *mut Value, ptr: *const u8, len: usize) -> *mut Value {
    unsafe {
        if local.is_null() {
            return error_value("invalid local");
        }
        if ptr.is_null() || len == 0 {
            return std::ptr::null_mut();
        }
        let local_ref = &mut *local;
        let bytes = std::slice::from_raw_parts(ptr, len);
        // Period source strings are valid UTF-8, so we can append the bytes
        // directly without the allocation/copy performed by `cstr`.
        let chunk = std::str::from_utf8_unchecked(bytes);
        match local_ref {
            Value::String(s) => {
                if s.is_empty() {
                    s.reserve(len.max(64));
                }
                s.push_str(chunk);
            }
            _ => {
                let mut s = chunk.to_string();
                s.reserve(len.max(64));
                let right = Value::String(s);
                let result = add_values(local_ref, &right);
                *local_ref = result;
            }
        }
        std::ptr::null_mut()
    }
}

/// Append a value to a local list in place when it already holds a list.
/// Otherwise fall back to list concatenation and assign the result.
/// Returns null on success so the caller does not need to drop a boxed Nothing.
#[unsafe(no_mangle)]
pub extern "C" fn period_append_local_list(local: *mut Value, item: *mut Value) -> *mut Value {
    unsafe {
        if local.is_null() || item.is_null() {
            if !item.is_null() {
                drop(Box::from_raw(item));
            }
            return error_value("invalid local or item");
        }
        let local_ref = &mut *local;
        let item_val = Box::from_raw(item);
        match local_ref {
            Value::List(list) => {
                if std::rc::Rc::strong_count(list) > 1 {
                    let mut new_items = list.borrow().clone();
                    new_items.push(*item_val);
                    *local_ref = Value::List(std::rc::Rc::new(std::cell::RefCell::new(new_items)));
                } else {
                    let mut borrowed = list.borrow_mut();
                    if borrowed.is_empty() {
                        borrowed.reserve(16);
                    }
                    borrowed.push(*item_val);
                }
            }
            _ => {
                let mut items = Vec::with_capacity(16);
                items.push(*item_val);
                let right = Value::List(std::rc::Rc::new(std::cell::RefCell::new(items)));
                let result = add_values(local_ref, &right);
                *local_ref = result;
            }
        }
        std::ptr::null_mut()
    }
}

/// Try to increment a local integer in place without allocation.
/// Returns 1 on success, 0 if the caller must fall back to generic addition.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_increment_local(local: *mut Value) -> i64 {
    unsafe {
        if local.is_null() {
            return 0;
        }
        match &mut *local {
            Value::Integer(Integer::Small(n)) => {
                if let Some(res) = n.checked_add(1) {
                    *n = res;
                    1
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

/// Try to add a source integer to a target local in place without allocation.
/// Returns 1 on success, 0 if the caller must fall back to generic addition.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_add_locals(target: *mut Value, source: *const Value) -> i64 {
    unsafe {
        if target.is_null() || source.is_null() {
            return 0;
        }
        match (&mut *target, &*source) {
            (Value::Integer(Integer::Small(t)), Value::Integer(Integer::Small(s))) => {
                if let Some(res) = t.checked_add(*s) {
                    *t = res;
                    1
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

fn sub_values(left: &Value, right: &Value) -> Value {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => Value::Integer(a.sub(b)),
        (Value::Integer(a), Value::Number(b)) => Value::Number(a.to_f64() - b),
        (Value::Number(a), Value::Integer(b)) => Value::Number(a - b.to_f64()),
        (Value::Number(a), Value::Number(b)) => Value::Number(a - b),
        _ => make_error(format!("cannot subtract {} and {}", left.type_name(), right.type_name())),
    }
}

fn mul_values(left: &Value, right: &Value) -> Value {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => Value::Integer(a.mul(b)),
        (Value::Integer(a), Value::Number(b)) => Value::Number(a.to_f64() * b),
        (Value::Number(a), Value::Integer(b)) => Value::Number(a * b.to_f64()),
        (Value::Number(a), Value::Number(b)) => Value::Number(a * b),
        (Value::String(s), Value::Integer(n)) | (Value::Integer(n), Value::String(s)) => {
            if n.to_bigint() < num_bigint::BigInt::from(0) {
                return make_error("Cannot repeat string a negative number of times".to_string());
            }
            let count = n.to_bigint().to_usize().unwrap_or(0);
            Value::String(s.repeat(count))
        }
        _ => make_error(format!("cannot multiply {} and {}", left.type_name(), right.type_name())),
    }
}

fn div_values(left: &Value, right: &Value) -> Value {
    if is_zero(right) {
        return make_error("Division by zero.".to_string());
    }
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => match a.div(b) {
            Some(f) => Value::Number(f),
            None => make_error("Division by zero.".to_string()),
        },
        (Value::Integer(a), Value::Number(b)) => Value::Number(a.to_f64() / b),
        (Value::Number(a), Value::Integer(b)) => Value::Number(a / b.to_f64()),
        (Value::Number(a), Value::Number(b)) => Value::Number(a / b),
        _ => make_error(format!("cannot divide {} by {}", left.type_name(), right.type_name())),
    }
}

fn mod_values(left: &Value, right: &Value) -> Value {
    if is_zero(right) {
        return make_error("modulo by zero".to_string());
    }
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => match a.modulo(b) {
            Some(n) => Value::Integer(n),
            None => make_error("modulo by zero".to_string()),
        },
        (Value::Number(a), Value::Number(b)) => Value::Number(a % b),
        (Value::Integer(a), Value::Number(b)) => Value::Number(a.to_f64() % b),
        (Value::Number(a), Value::Integer(b)) => Value::Number(a % b.to_f64()),
        _ => make_error(format!("cannot modulo {} by {}", left.type_name(), right.type_name())),
    }
}

fn pow_values(left: &Value, right: &Value) -> Value {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => match a.pow(b) {
            Ok(n) => Value::Integer(n),
            Err(e) => make_error(e.to_string()),
        },
        (Value::Integer(a), Value::Number(b)) => Value::Number(a.to_f64().powf(*b)),
        (Value::Number(a), Value::Integer(b)) => Value::Number(a.powf(b.to_f64())),
        (Value::Number(a), Value::Number(b)) => Value::Number(a.powf(*b)),
        _ => make_error(format!("cannot exponentiate {} by {}", left.type_name(), right.type_name())),
    }
}

fn cmp_values(op: BinOp, left: &Value, right: &Value) -> Value {
    let ord = match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => Some(a.cmp(b)),
        (Value::Integer(a), Value::Number(b)) => a.cmp_integer_f64(*b),
        (Value::Number(a), Value::Integer(b)) => b.cmp_integer_f64(*a).map(|o| o.reverse()),
        (Value::Number(a), Value::Number(b)) => a.partial_cmp(b),
        (Value::String(a), Value::String(b)) => a.partial_cmp(b),
        _ => None,
    };
    let result = match ord {
        Some(ordering) => match op {
            BinOp::Lt => ordering == std::cmp::Ordering::Less,
            BinOp::Le => ordering != std::cmp::Ordering::Greater,
            BinOp::Gt => ordering == std::cmp::Ordering::Greater,
            BinOp::Ge => ordering != std::cmp::Ordering::Less,
            BinOp::Eq => ordering == std::cmp::Ordering::Equal,
            BinOp::Ne => ordering != std::cmp::Ordering::Equal,
            _ => false,
        },
        None => false,
    };
    Value::Bool(result)
}

fn is_zero(v: &Value) -> bool {
    match v {
        Value::Integer(n) => n.is_zero(),
        Value::Number(f) => *f == 0.0,
        _ => false,
    }
}

/// Unary operations.
#[unsafe(no_mangle)]
pub extern "C" fn period_value_unary(op: UnaryOp, operand: *const Value) -> *mut Value {
    unsafe {
        let operand = match operand.as_ref() {
            Some(v) => v,
            None => return error_value("missing unary operand"),
        };
        let result = match op {
            UnaryOp::Neg => match operand {
                Value::Integer(n) => Value::Integer(n.neg()),
                Value::Number(f) => Value::Number(-f),
                _ => make_error(format!("cannot negate {}", operand.type_name())),
            },
            UnaryOp::Not => match operand {
                Value::Bool(b) => Value::Bool(!b),
                _ => make_error("'not' requires a boolean operand".to_string()),
            },
        };
        Box::into_raw(Box::new(result))
    }
}

// ---------------------------------------------------------------------------
// Globals
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn period_env_get(interp: *mut Interpreter, name_ptr: *const u8, name_len: usize) -> *mut Value {
    unsafe {
        let interp = match interp.as_ref() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let name = cstr(name_ptr, name_len);
        match interp.env.borrow().get(&name) {
            Some(v) => Box::into_raw(Box::new(v)),
            None => error_value(format!("Undefined variable '{}'", name)),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_env_set(interp: *mut Interpreter, name_ptr: *const u8, name_len: usize, value: *mut Value) -> *mut Value {
    unsafe {
        let interp = match interp.as_mut() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let name = cstr(name_ptr, name_len);
        let value = take_value(value);
        match interp.env.borrow().set(&name, value) {
            Ok(()) => period_value_nothing(),
            Err(msg) => error_value(msg),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_env_define(
    interp: *mut Interpreter,
    name_ptr: *const u8,
    name_len: usize,
    value: *mut Value,
    ann_ptr: *const u8,
    ann_len: usize,
) -> *mut Value {
    unsafe {
        let interp = match interp.as_mut() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let name = cstr(name_ptr, name_len);
        let value = take_value(value);
        let ann = if ann_len == 0 { None } else { Some(cstr(ann_ptr, ann_len)) };
        interp.env.borrow().define(&name, value, ann);
        period_value_nothing()
    }
}

// ---------------------------------------------------------------------------
// Composite values
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn period_build_list(argc: usize, argv: *const *mut Value) -> *mut Value {
    let items = argv_to_vec(argc, argv);
    for p in unsafe { std::slice::from_raw_parts(argv, argc) } {
        if !p.is_null() {
            unsafe { drop(Box::from_raw(*p)); }
        }
    }
    Box::into_raw(Box::new(Value::List(std::rc::Rc::new(std::cell::RefCell::new(items)))))
}

#[unsafe(no_mangle)]
pub extern "C" fn period_build_dict(pairc: usize, kv: *const *mut Value) -> *mut Value {
    unsafe {
        let mut map = std::collections::HashMap::new();
        let slice = std::slice::from_raw_parts(kv, pairc * 2);
        for i in 0..pairc {
            let key = take_value(slice[i * 2]);
            let value = take_value(slice[i * 2 + 1]);
            let key = match key.as_key() {
                Ok(k) => k,
                Err(msg) => return error_value(msg),
            };
            map.insert(key, value);
        }
        Box::into_raw(Box::new(Value::Dict(std::rc::Rc::new(std::cell::RefCell::new(map)))))
    }
}

fn as_index(idx: &Value, len: usize) -> Result<usize, String> {
    let n = match idx {
        Value::Integer(i) => i.to_bigint(),
        Value::Number(f) if f.fract() == 0.0 => num_bigint::BigInt::from_f64(*f).unwrap_or_else(|| num_bigint::BigInt::from(0)),
        _ => return Err(format!("Index must be an integer, got {}", idx.type_name())),
    };
    let len_bi = num_bigint::BigInt::from(len as i64);
    let i = if n.sign() == num_bigint::Sign::Minus {
        &len_bi + n
    } else {
        n
    };
    if i < num_bigint::BigInt::from(0) || i >= len_bi {
        Err("Index out of range".to_string())
    } else {
        Ok(i.to_usize().unwrap_or(0))
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_index_get(obj: *mut Value, idx: *mut Value) -> *mut Value {
    let obj = take_value(obj);
    let idx = take_value(idx);
    let result = match &obj {
        Value::List(list) => {
            let len = list.borrow().len();
            match as_index(&idx, len) {
                Ok(i) => list.borrow().get(i).cloned().unwrap_or(Value::Nothing),
                Err(msg) => return error_value(msg),
            }
        }
        Value::Dict(dict) => {
            let key = match idx.as_key() {
                Ok(k) => k,
                Err(msg) => return error_value(msg),
            };
            dict.borrow().get(&key).cloned().unwrap_or(Value::Nothing)
        }
        Value::String(s) => {
            let len = s.len();
            match as_index(&idx, len) {
                Ok(i) => s.chars().nth(i).map(|c| Value::String(c.to_string())).unwrap_or(Value::Nothing),
                Err(msg) => return error_value(msg),
            }
        }
        Value::Range { start, stop, step } => {
            let len = crate::value::range_len(*start, *stop, *step) as usize;
            match as_index(&idx, len) {
                Ok(i) => Value::Integer(Integer::Small(*start + (*step) * (i as i64))),
                Err(msg) => return error_value(msg),
            }
        }
        _ => return error_value(format!("Cannot index {}", obj.type_name())),
    };
    Box::into_raw(Box::new(result))
}

#[unsafe(no_mangle)]
pub extern "C" fn period_index_set(obj: *mut Value, idx: *mut Value, value: *mut Value) -> *mut Value {
    let obj = take_value(obj);
    let idx = take_value(idx);
    let value = take_value(value);
    match &obj {
        Value::List(list) => {
            let len = list.borrow().len();
            match as_index(&idx, len) {
                Ok(i) => list.borrow_mut()[i] = value,
                Err(msg) => return error_value(msg),
            }
        }
        Value::Dict(dict) => {
            let key = match idx.as_key() {
                Ok(k) => k,
                Err(msg) => return error_value(msg),
            };
            dict.borrow_mut().insert(key, value);
        }
        _ => return error_value(format!("Cannot index-assign {}", obj.type_name())),
    }
    Box::into_raw(Box::new(obj))
}

fn field_name_matches(name_ptr: *const u8, name_len: usize, s: &str) -> bool {
    if name_len != s.len() {
        return false;
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(name_ptr, name_len);
        bytes == s.as_bytes()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_property_get(obj: *mut Value, name_ptr: *const u8, name_len: usize) -> *mut Value {
    let obj = take_value(obj);
    // Fast path: if the instance has a fixed slot layout, look up by byte comparison
    // without allocating a String.
    if let Value::Instance { slots, class, .. } = &obj {
        if let Some(slots) = slots {
            if let Value::VMClass(cv) = class.as_ref() {
                if let Some(idx) = cv.field_names.iter().position(|n| field_name_matches(name_ptr, name_len, n)) {
                    return Box::into_raw(Box::new(slots.borrow().get(idx).cloned().unwrap_or(Value::Nothing)));
                }
            }
        }
    }
    let name = cstr(name_ptr, name_len);
    let result = match &obj {
        Value::Instance { fields, .. } => {
            if let Some(fields) = fields {
                fields.borrow().get(&name).cloned().unwrap_or(Value::Nothing)
            } else {
                Value::Nothing
            }
        }
        Value::Error(ev) => match name.as_str() {
            "message" => Value::String(ev.message.clone()),
            "line" => Value::Integer(Integer::Small(ev.line as i64)),
            "col" => Value::Integer(Integer::Small(ev.col as i64)),
            _ => Value::Nothing,
        },
        Value::Module(mv) => mv.env.borrow().get(&name).unwrap_or(Value::Nothing),
        _ => return error_value(format!("Cannot get property '{}' of {}", name, obj.type_name())),
    };
    Box::into_raw(Box::new(result))
}

#[unsafe(no_mangle)]
pub extern "C" fn period_property_set(obj: *mut Value, name_ptr: *const u8, name_len: usize, value: *mut Value) -> *mut Value {
    let obj = take_value(obj);
    let value = take_value(value);
    // Fast path for fixed-slot instances: compare names by bytes, no String allocation.
    if let Value::Instance { ref slots, ref class, .. } = obj {
        if let Some(slots) = slots {
            if let Value::VMClass(cv) = class.as_ref() {
                if let Some(idx) = cv.field_names.iter().position(|n| field_name_matches(name_ptr, name_len, n)) {
                    slots.borrow_mut()[idx] = value;
                    return Box::into_raw(Box::new(obj));
                }
            }
        }
    }
    let name = cstr(name_ptr, name_len);
    match obj {
        Value::Instance { ref fields, .. } => {
            if let Some(fields) = fields {
                fields.borrow_mut().insert(name, value);
            } else {
                return error_value(format!("Cannot set property on {}", obj.type_name()));
            }
        }
        _ => return error_value(format!("Cannot set property on {}", obj.type_name())),
    }
    Box::into_raw(Box::new(obj))
}

#[unsafe(no_mangle)]
pub extern "C" fn period_length(v: *mut Value) -> *mut Value {
    let v = take_value(v);
    let len = match &v {
        Value::String(s) => s.len() as i64,
        Value::List(l) => l.borrow().len() as i64,
        Value::Dict(d) => d.borrow().len() as i64,
        Value::Range { start, stop, step } => crate::value::range_len(*start, *stop, *step),
        _ => return error_value(format!("length not supported for {}", v.type_name())),
    };
    Box::into_raw(Box::new(Value::Integer(Integer::Small(len))))
}

#[unsafe(no_mangle)]
pub extern "C" fn period_iter_init(v: *mut Value) -> *mut Value {
    let v = take_value(v);
    let items = match v {
        Value::List(l) => l.borrow().clone(),
        Value::String(s) => s.chars().map(|c| Value::String(c.to_string())).collect(),
        Value::Dict(d) => d.borrow().keys().map(|k| k.to_value()).collect(),
        Value::Range { start, stop, step } => {
            let mut out = Vec::new();
            let mut i = start;
            if step > 0 {
                while i < stop { out.push(Value::Integer(Integer::Small(i))); i += step; }
            } else if step < 0 {
                while i > stop { out.push(Value::Integer(Integer::Small(i))); i += step; }
            }
            out
        }
        _ => return error_value(format!("Cannot iterate over {}", v.type_name())),
    };
    Box::into_raw(Box::new(Value::List(std::rc::Rc::new(std::cell::RefCell::new(items)))))
}

// ---------------------------------------------------------------------------
// Calls, classes, closures
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn period_call(interp: *mut Interpreter, callee: *mut Value, argc: usize, argv: *const *mut Value) -> *mut Value {
    unsafe {
        let interp = match interp.as_mut() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let (line, col) = current_span();
        let span = crate::ast::Span { line: line as usize, col: col as usize };
        let callee = take_value(callee);
        match &callee {
            Value::VMClass(_) => {
                let args = argv_to_vec(argc, argv);
                for p in std::slice::from_raw_parts(argv, argc) {
                    if !p.is_null() {
                        drop(Box::from_raw(*p));
                    }
                }
                result_to_ptr(new_instance(interp, callee, args, &span))
            }
            Value::VMFunction(fv) => {
                if fv.func.params.len() != argc {
                    for p in std::slice::from_raw_parts(argv, argc) {
                        if !p.is_null() {
                            drop(Box::from_raw(*p));
                        }
                    }
                    return error_value(format!(
                        "Function {} expects {} args, got {}",
                        fv.func.name,
                        fv.func.params.len(),
                        argc
                    ));
                }
                if let Some(code) = crate::jit_generic::get_jit_code(&fv.func) {
                    let upvalues_ptr = if fv.upvalues.is_empty() {
                        std::ptr::null_mut()
                    } else {
                        let upvalues: Vec<*const std::ffi::c_void> = fv
                            .upvalues
                            .iter()
                            .map(|rc| Rc::as_ptr(rc) as *const std::ffi::c_void)
                            .collect();
                        upvalues.as_ptr() as *mut _
                    };
                    let ctx = crate::jit_generic::JitContext {
                        interp,
                        function: &*fv.func as *const CompiledFunction,
                    };
                    let result = code(
                        &ctx as *const _ as *mut std::ffi::c_void,
                        upvalues_ptr,
                        argc,
                        argv,
                    );
                    return result;
                }
                let args = argv_to_vec(argc, argv);
                for p in std::slice::from_raw_parts(argv, argc) {
                    if !p.is_null() {
                        drop(Box::from_raw(*p));
                    }
                }
                result_to_ptr(interp.call_value(&callee, args, &span))
            }
            _ => {
                let args = argv_to_vec(argc, argv);
                for p in std::slice::from_raw_parts(argv, argc) {
                    if !p.is_null() {
                        drop(Box::from_raw(*p));
                    }
                }
                result_to_ptr(interp.call_value(&callee, args, &span))
            }
        }
    }
}

fn new_instance(interp: &mut Interpreter, cls: Value, args: Vec<Value>, span: &crate::ast::Span) -> Result<Value, Control> {
    match cls {
        Value::VMClass(ref cv) => {
            let has_layout = !cv.field_names.is_empty();
            let slots = if has_layout {
                Some(std::rc::Rc::new(std::cell::RefCell::new(vec![Value::Nothing; cv.field_names.len()])))
            } else {
                None
            };
            let simple_init = has_layout && cv.field_init.iter().all(|m| m.is_some());
            let instance = Value::Instance {
                class: Box::new(cls.clone()),
                fields: if simple_init { None } else { Some(std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new()))) },
                slots: slots.clone(),
            };
            if simple_init {
                if let Some(ref slots) = slots {
                    let mut slot_vec = slots.borrow_mut();
                    for (field_idx, param_idx) in cv.field_init.iter().enumerate() {
                        let param_idx = param_idx.unwrap();
                        if param_idx < args.len() {
                            slot_vec[field_idx] = args[param_idx].clone();
                        }
                    }
                }
            } else if let Some(init_value) = &cv.init {
                let mut init_args = vec![instance.clone()];
                init_args.extend(args);
                interp.call_value(&(**init_value), init_args, span)?;
            } else if !args.is_empty() {
                return Err(Control::RuntimeError("Class takes no init arguments".to_string(), span.clone()));
            }
            Ok(instance)
        }
        Value::Class(_) => interp.call_value(&cls, args, span),
        _ => Err(Control::RuntimeError(format!("Cannot create instance of {}", cls.type_name()), span.clone())),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_new_instance(
    interp: *mut Interpreter,
    cls: *mut Value,
    argc: usize,
    argv: *const *mut Value,
) -> *mut Value {
    unsafe {
        let interp = match interp.as_mut() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let (line, col) = current_span();
        let span = crate::ast::Span { line: line as usize, col: col as usize };
        let cls = take_value(cls);
        let args = argv_to_vec(argc, argv);
        for p in std::slice::from_raw_parts(argv, argc) {
            if !p.is_null() {
                drop(Box::from_raw(*p));
            }
        }
        result_to_ptr(new_instance(interp, cls, args, &span))
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_tell(
    interp: *mut Interpreter,
    obj: *mut Value,
    name_ptr: *const u8,
    name_len: usize,
    argc: usize,
    argv: *const *mut Value,
) -> *mut Value {
    unsafe {
        let interp = match interp.as_mut() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let (line, col) = current_span();
        let span = crate::ast::Span { line: line as usize, col: col as usize };
        let obj = take_value(obj);
        let name = cstr(name_ptr, name_len);
        let mut args = argv_to_vec(argc, argv);
        for p in std::slice::from_raw_parts(argv, argc) {
            if !p.is_null() {
                drop(Box::from_raw(*p));
            }
        }
        let class = match &obj {
            Value::Instance { class, .. } => class.clone(),
            _ => return error_value(format!("Cannot send message to {}", obj.type_name())),
        };
        let result = match class.as_ref() {
            Value::VMClass(cv) => {
                let method = match cv.methods.get(&name) {
                    Some(m) => m.clone(),
                    None => return error_value(format!("Unknown method '{}' on {}", name, cv.name)),
                };
                let mut call_args = vec![obj];
                call_args.append(&mut args);
                interp.call_value(&method, call_args, &span)
            }
            _ => return error_value(format!("Cannot send message to {}", obj.type_name())),
        };
        result_to_ptr(result)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_make_closure(
    interp: *mut Interpreter,
    function: *const CompiledFunction,
    func_idx: usize,
    upvalue_count: usize,
    upvalues: *const *mut std::ffi::c_void,
) -> *mut Value {
    unsafe {
        let interp = match interp.as_ref() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let function = match function.as_ref() {
            Some(f) => f,
            None => return error_value("invalid function"),
        };
        let proto = match function.chunk.functions.get(func_idx) {
            Some(f) => f.clone(),
            None => return error_value("closure function index out of range"),
        };
        let upvalues = if upvalue_count == 0 || upvalues.is_null() {
            Vec::new()
        } else {
            let slice = std::slice::from_raw_parts(upvalues, upvalue_count);
            slice
                .iter()
                .map(|&ptr| {
                    let rc = Rc::from_raw(ptr as *const RefCell<Value>);
                    let clone = rc.clone();
                    let _ = Rc::into_raw(rc);
                    clone
                })
                .collect()
        };
        let closure = Value::VMFunction(Box::new(VMFunctionValue {
            func: proto,
            closure: interp.env.clone(),
            upvalues,
            from_module: interp.loading_module,
        }));
        Box::into_raw(Box::new(closure))
    }
}

/// Allocate a heap cell that can be shared between a local variable and a closure.
#[unsafe(no_mangle)]
pub extern "C" fn period_upvalue_alloc(value: *mut Value) -> *mut std::ffi::c_void {
    let value = take_value(value);
    let cell = Rc::new(RefCell::new(value));
    Rc::into_raw(cell) as *mut std::ffi::c_void
}

/// Read the current value from a shared upvalue cell.
#[unsafe(no_mangle)]
pub extern "C" fn period_upvalue_get(cell: *mut std::ffi::c_void) -> *mut Value {
    unsafe {
        if cell.is_null() {
            return std::ptr::null_mut();
        }
        let cell = cell as *const RefCell<Value>;
        let value = (*cell).borrow().clone();
        Box::into_raw(Box::new(value))
    }
}

/// Replace the value stored in a shared upvalue cell.
#[unsafe(no_mangle)]
pub extern "C" fn period_upvalue_set(cell: *mut std::ffi::c_void, value: *mut Value) {
    unsafe {
        if cell.is_null() || value.is_null() {
            return;
        }
        let cell = cell as *const RefCell<Value>;
        let new_value = take_value(value);
        let old_value = std::mem::replace(&mut *(*cell).borrow_mut(), new_value);
        drop(old_value);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_build_class(
    interp: *mut Interpreter,
    name_ptr: *const u8,
    name_len: usize,
    init: *mut Value,
    method_count: usize,
    methods: *const *mut Value,
    method_names: *const *const u8,
    method_name_lens: *const usize,
    field_count: usize,
    field_names: *const *const u8,
    field_name_lens: *const usize,
    field_init: *const usize,
) -> *mut Value {
    unsafe {
        let interp = match interp.as_ref() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let name = cstr(name_ptr, name_len);
        let mut method_map = std::collections::HashMap::new();
        let method_slice = std::slice::from_raw_parts(methods, method_count);
        let name_ptrs = std::slice::from_raw_parts(method_names, method_count);
        let name_lens = std::slice::from_raw_parts(method_name_lens, method_count);
        for i in 0..method_count {
            let method = take_value(method_slice[i]);
            let mname = cstr(name_ptrs[i], name_lens[i]);
            method_map.insert(mname, method);
        }
        let init = if init.is_null() { None } else { Some(Box::new(take_value(init))) };
        let from_module = interp.loading_module;
        let mut field_name_vec = Vec::with_capacity(field_count);
        let field_name_ptrs = std::slice::from_raw_parts(field_names, field_count);
        let field_name_lens_slice = std::slice::from_raw_parts(field_name_lens, field_count);
        for i in 0..field_count {
            field_name_vec.push(cstr(field_name_ptrs[i], field_name_lens_slice[i]));
        }
        let field_init_slice = std::slice::from_raw_parts(field_init, field_count);
        let field_init_vec: Vec<Option<usize>> = field_init_slice.iter().map(|&i| if i == usize::MAX { None } else { Some(i) }).collect();
        Box::into_raw(Box::new(Value::VMClass(Box::new(VMClassValue {
            name,
            init,
            methods: method_map,
            field_names: field_name_vec,
            field_init: field_init_vec,
            from_module,
        }))))
    }
}

// ---------------------------------------------------------------------------
// Modules / I/O / types
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn period_import(interp: *mut Interpreter, path_ptr: *const u8, path_len: usize) -> *mut Value {
    unsafe {
        let interp = match interp.as_mut() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let (line, col) = current_span();
        let span = crate::ast::Span { line: line as usize, col: col as usize };
        let path = cstr(path_ptr, path_len);
        result_to_ptr(interp.import_module(&path, &span).map(|()| Value::Nothing))
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_qualified_get(
    interp: *mut Interpreter,
    mod_ptr: *const u8,
    mod_len: usize,
    name_ptr: *const u8,
    name_len: usize,
) -> *mut Value {
    unsafe {
        let interp = match interp.as_ref() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let module = cstr(mod_ptr, mod_len);
        let name = cstr(name_ptr, name_len);
        let result = match interp.modules.borrow().get(&module).cloned() {
            Some(env) => env.borrow().get(&name).unwrap_or(Value::Nothing),
            None => return error_value(format!("Module '{}' not imported", module)),
        };
        Box::into_raw(Box::new(result))
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_export_name(interp: *mut Interpreter, name_ptr: *const u8, name_len: usize) -> *mut Value {
    unsafe {
        let interp = match interp.as_ref() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let name = cstr(name_ptr, name_len);
        interp.env.borrow().add_export(&name);
        period_value_nothing()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_read(interp: *mut Interpreter, path_value: *mut Value) -> *mut Value {
    unsafe {
        let interp = match interp.as_ref() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let path_value = take_value(path_value);
        let path = match &path_value {
            Value::String(s) => s.clone(),
            _ => return error_value(format!("Path must be a string, got {}", path_value.type_name())),
        };
        let full = interp.resolve_path(&path);
        match std::fs::read_to_string(&full) {
            Ok(s) => Box::into_raw(Box::new(Value::String(s))),
            Err(e) => error_value(format!("Cannot read '{}': {}", full.display(), e)),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_write(
    interp: *mut Interpreter,
    path_value: *mut Value,
    content_value: *mut Value,
) -> *mut Value {
    unsafe {
        let interp = match interp.as_ref() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let path_value = take_value(path_value);
        let content_value = take_value(content_value);
        let path = match &path_value {
            Value::String(s) => s.clone(),
            _ => return error_value(format!("Path must be a string, got {}", path_value.type_name())),
        };
        let content = content_value.to_string();
        let full = interp.resolve_path(&path);
        match std::fs::write(&full, content) {
            Ok(()) => period_value_nothing(),
            Err(e) => error_value(format!("Cannot write '{}': {}", full.display(), e)),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn period_check_type(
    interp: *mut Interpreter,
    value: *const Value,
    ann_ptr: *const u8,
    ann_len: usize,
) -> *mut Value {
    unsafe {
        let interp = match interp.as_mut() {
            Some(i) => i,
            None => return error_value("invalid interpreter"),
        };
        let value = match value.as_ref() {
            Some(v) => v,
            None => return period_value_nothing(),
        };
        let ann = cstr(ann_ptr, ann_len);
        match interp.check_type(value, &ann, &crate::ast::Span { line: 0, col: 0 }) {
            Ok(()) => period_value_nothing(),
            Err(Control::RuntimeError(msg, _)) | Err(Control::Error(msg)) => error_value(msg),
            Err(_) => error_value("type check failed"),
        }
    }
}

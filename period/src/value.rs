use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use num_bigint::BigInt;
use num_traits::cast::{FromPrimitive, ToPrimitive};
use num_traits::Zero;

use crate::environment::Environment;

/// Arbitrary-precision integer with a fast path for values that fit in i64.
#[derive(Clone, Debug)]
pub enum Integer {
    Small(i64),
    Big(BigInt),
}

impl PartialEq for Integer {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Integer::Small(a), Integer::Small(b)) => a == b,
            (Integer::Big(a), Integer::Big(b)) => a == b,
            _ => self.to_bigint() == other.to_bigint(),
        }
    }
}

impl Eq for Integer {}

impl std::hash::Hash for Integer {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.to_bigint().hash(state);
    }
}

impl Integer {
    pub fn from_i64(n: i64) -> Self {
        Integer::Small(n)
    }

    pub fn from_bigint(n: BigInt) -> Self {
        n.to_i64().map(Integer::Small).unwrap_or(Integer::Big(n))
    }

    pub fn to_i64(&self) -> Option<i64> {
        match self {
            Integer::Small(n) => Some(*n),
            Integer::Big(n) => n.to_i64(),
        }
    }

    pub fn to_f64(&self) -> f64 {
        match self {
            Integer::Small(n) => *n as f64,
            Integer::Big(n) => n.to_f64().unwrap_or(0.0),
        }
    }

    pub fn to_bigint(&self) -> BigInt {
        match self {
            Integer::Small(n) => BigInt::from(*n),
            Integer::Big(n) => n.clone(),
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Integer::Small(n) => *n == 0,
            Integer::Big(n) => n.is_zero(),
        }
    }

    pub fn neg(&self) -> Self {
        match self {
            Integer::Small(n) => {
                if let Some(res) = n.checked_neg() {
                    Integer::Small(res)
                } else {
                    Integer::from_bigint(-self.to_bigint())
                }
            }
            Integer::Big(n) => Integer::from_bigint(-n),
        }
    }

    /// Add two integers. Returns the result as a Value::Integer or Value::Number if
    /// the operation would overflow the arbitrary-precision representation (not
    /// applicable here, but kept for symmetry with other numeric ops).
    pub fn add(&self, other: &Integer) -> Self {
        match (self, other) {
            (Integer::Small(a), Integer::Small(b)) => {
                if let Some(res) = a.checked_add(*b) {
                    Integer::Small(res)
                } else {
                    Integer::from_bigint(self.to_bigint() + other.to_bigint())
                }
            }
            _ => Integer::from_bigint(self.to_bigint() + other.to_bigint()),
        }
    }

    pub fn sub(&self, other: &Integer) -> Self {
        match (self, other) {
            (Integer::Small(a), Integer::Small(b)) => {
                if let Some(res) = a.checked_sub(*b) {
                    Integer::Small(res)
                } else {
                    Integer::from_bigint(self.to_bigint() - other.to_bigint())
                }
            }
            _ => Integer::from_bigint(self.to_bigint() - other.to_bigint()),
        }
    }

    pub fn mul(&self, other: &Integer) -> Self {
        match (self, other) {
            (Integer::Small(a), Integer::Small(b)) => {
                if let Some(res) = a.checked_mul(*b) {
                    Integer::Small(res)
                } else {
                    Integer::from_bigint(self.to_bigint() * other.to_bigint())
                }
            }
            _ => Integer::from_bigint(self.to_bigint() * other.to_bigint()),
        }
    }

    pub fn add_assign(&mut self, other: &Integer) {
        if let (Integer::Small(a), Integer::Small(b)) = (&*self, other) {
            if let Some(res) = a.checked_add(*b) {
                *self = Integer::Small(res);
                return;
            }
        }
        *self = Integer::from_bigint(self.to_bigint() + other.to_bigint());
    }

    pub fn mul_assign(&mut self, other: &Integer) {
        if let (Integer::Small(a), Integer::Small(b)) = (&*self, other) {
            if let Some(res) = a.checked_mul(*b) {
                *self = Integer::Small(res);
                return;
            }
        }
        *self = Integer::from_bigint(self.to_bigint() * other.to_bigint());
    }

    #[allow(dead_code)]
    pub fn div(&self, other: &Integer) -> Option<f64> {
        if other.is_zero() {
            return None;
        }
        Some(self.to_f64() / other.to_f64())
    }

    pub fn modulo(&self, other: &Integer) -> Option<Self> {
        if other.is_zero() {
            return None;
        }
        Some(Integer::from_bigint(self.to_bigint() % other.to_bigint()))
    }

    pub fn pow(&self, other: &Integer) -> Result<Self, &'static str> {
        if other.is_zero() {
            return Ok(Integer::Small(1));
        }
        if self.is_zero() && other.to_bigint() < BigInt::from(0) {
            return Err("Division by zero");
        }
        let exp = other.to_bigint();
        if exp < BigInt::from(0) {
            return Ok(Integer::Small(0));
        }
        let exp_u32 = exp.to_u32().ok_or("Exponent too large")?;
        Ok(Integer::from_bigint(self.to_bigint().pow(exp_u32)))
    }

    pub fn cmp_integer_f64(&self, number: f64) -> Option<std::cmp::Ordering> {
        if !number.is_finite() {
            return None;
        }
        if number.fract() == 0.0 {
            if let Some(i) = BigInt::from_f64(number) {
                return Some(self.to_bigint().cmp(&i));
            }
        }
        let ord = self.to_f64().partial_cmp(&number).unwrap_or(std::cmp::Ordering::Equal);
        if ord == std::cmp::Ordering::Equal {
            Some(if number.fract() > 0.0 { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater })
        } else {
            Some(ord)
        }
    }
}

impl PartialOrd for Integer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Integer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Integer::Small(a), Integer::Small(b)) => a.cmp(b),
            _ => self.to_bigint().cmp(&other.to_bigint()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClassValue {
    pub name: String,
    pub init: Option<Box<Value>>,
    pub methods: HashMap<String, Value>,
    pub field_names: Vec<String>,
    pub field_init: Vec<Option<usize>>,
    #[allow(dead_code)]
    pub from_module: bool,
}

#[derive(Clone)]
pub struct FunctionValue {
    pub func: Rc<crate::bytecode::CompiledFunction>,
    pub closure: Rc<RefCell<Environment>>,
    pub upvalues: Vec<Rc<RefCell<Value>>>,
    #[allow(dead_code)]
    pub from_module: bool,
}

#[derive(Clone, Debug)]
pub struct BuiltInValue {
    pub name: String,
    pub min_arity: usize,
    pub max_arity: usize,
    pub func: fn(&[Value]) -> Result<Value, String>,
}

#[derive(Clone)]
pub struct ModuleValue {
    pub name: String,
    pub env: Rc<RefCell<Environment>>,
}

#[derive(Clone, Debug)]
pub struct ErrorValue {
    pub message: String,
    pub line: i64,
    pub col: i64,
}

#[derive(Clone)]
pub enum Value {
    Integer(Integer),
    Number(f64),
    String(String),
    Bool(bool),
    Nothing,
    List(Rc<RefCell<Vec<Value>>>),
    Dict(Rc<RefCell<HashMap<ValueKey, Value>>>),
    Range {
        start: Integer,
        stop: Integer,
        step: Integer,
    },
    Function(Box<FunctionValue>),
    Instance {
        class: Box<Value>,
        fields: Option<Rc<RefCell<HashMap<String, Value>>>>,
        slots: Option<Rc<RefCell<Vec<Value>>>>,
    },
    BuiltIn(Box<BuiltInValue>),
    Class(Box<ClassValue>),
    Module(Box<ModuleValue>),
    Error(Box<ErrorValue>),
    Box(Rc<RefCell<Value>>),
}

impl Value {
    pub fn integer(n: i64) -> Self {
        Value::Integer(Integer::Small(n))
    }

    pub fn big_integer(n: BigInt) -> Self {
        Value::Integer(Integer::from_bigint(n))
    }
}

/// Dict keys in deterministic (text-sorted) order. HashMap iteration order is
/// not stable across processes, so every backend iterates dict keys through
/// this helper to keep `for` loops over dictionaries reproducible.
pub fn dict_sorted_keys(d: &HashMap<ValueKey, Value>) -> Vec<Value> {
    let mut keys: Vec<Value> = d.keys().map(|k| k.to_value()).collect();
    keys.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
    keys
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Integer(n) => write!(f, "{}", n.to_bigint()),
            Value::Number(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            Value::Nothing => write!(f, "nothing"),
            Value::List(l) => write!(f, "{:?}", l.borrow()),
            Value::Dict(d) => {
                let mut items: Vec<String> = d.borrow().iter()
                    .map(|(k, v)| format!("{:?}: {:?}", k.to_value(), v))
                    .collect();
                items.sort();
                write!(f, "{{{}}}", items.join(", "))
            }
            Value::Function(fv) => write!(f, "<function {}>", fv.func.name),
            Value::Class(cv) => write!(f, "<class {}>", cv.name),
            Value::Instance { class, .. } => write!(f, "<instance of {:?}>", class),
            Value::BuiltIn(bv) => write!(f, "<built-in {}>", bv.name),
            Value::Module(mv) => write!(f, "<module {}>", mv.name),
            Value::Error(ev) => write!(f, "error: {}", ev.message),
            Value::Range { start, stop, step } => write!(f, "range({}, {}, {})", start.to_bigint(), stop.to_bigint(), step.to_bigint()),
            Value::Box(v) => write!(f, "{:?}", v.borrow()),
        }
    }
}

fn integer_eq_f64(a: &Integer, b: f64) -> bool {
    if !b.is_finite() {
        return false;
    }
    if b.fract() != 0.0 {
        return false;
    }
    if let Some(i) = BigInt::from_f64(b) {
        a.to_bigint() == i
    } else {
        false
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Integer(a), Value::Number(b)) | (Value::Number(b), Value::Integer(a)) => integer_eq_f64(a, *b),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Nothing, Value::Nothing) => true,
            (Value::List(a), Value::List(b)) => a.borrow().eq(&*b.borrow()),
            (Value::Dict(a), Value::Dict(b)) => a.borrow().eq(&*b.borrow()),
            (Value::Error(a), Value::Error(b)) => a.message == b.message,
            (
                Value::Range { start: a, stop: b, step: c },
                Value::Range { start: d, stop: e, step: f },
            ) => a == d && b == e && c == f,
            (Value::Box(a), other) => a.borrow().eq(other),
            (other, Value::Box(b)) => other.eq(&*b.borrow()),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueKey {
    Integer(BigInt),
    Number(u64),
    String(String),
    Bool(bool),
    Nothing,
}

impl Value {
    pub fn as_key(&self) -> Result<ValueKey, String> {
        match self {
            Value::Integer(n) => Ok(ValueKey::Integer(n.to_bigint())),
            Value::Number(n) => Ok(ValueKey::Number(n.to_bits())),
            Value::String(s) => Ok(ValueKey::String(s.clone())),
            Value::Bool(b) => Ok(ValueKey::Bool(*b)),
            Value::Nothing => Ok(ValueKey::Nothing),
            Value::Range { .. } => Err("range is not hashable".to_string()),
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
            Value::Function(_) => "function",
            Value::Class(_) => "class",
            Value::Instance { .. } => "instance",
            Value::BuiltIn(_) => "built-in",
            Value::Module(_) => "module",
            Value::Range { .. } => "range",
            Value::Error(_) => "error",
            Value::Box(v) => v.borrow().type_name(),
        }
    }

}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Integer(n) => write!(f, "{}", n.to_bigint()),
            Value::Number(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            Value::Nothing => write!(f, "nothing"),
            Value::List(l) => {
                let items: Vec<String> = l.borrow().iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", items.join(", "))
            }
            Value::Dict(d) => {
                // Sorted by key text so output is deterministic across runs
                // and execution backends (HashMap iteration order is not).
                let mut items: Vec<String> = d.borrow().iter()
                    .map(|(k, v)| format!("{}: {}", k.to_value(), v))
                    .collect();
                items.sort();
                write!(f, "{{{}}}", items.join(", "))
            }
            Value::Function(fv) => write!(f, "<function {}>", fv.func.name),
            Value::Class(cv) => write!(f, "<class {}>", cv.name),
            Value::Instance { class, .. } => write!(f, "<instance of {:?}>", class),
            Value::BuiltIn(bv) => write!(f, "<built-in {}>", bv.name),
            Value::Module(mv) => write!(f, "<module {}>", mv.name),
            Value::Range { start, stop, step } => write!(f, "range({}, {}, {})", start.to_bigint(), stop.to_bigint(), step.to_bigint()),
            Value::Error(ev) => write!(f, "{}:{}: {}", ev.line, ev.col, ev.message),
            Value::Box(v) => write!(f, "{}", v.borrow()),
        }
    }
}

impl ValueKey {
    pub fn to_value(&self) -> Value {
        match self {
            ValueKey::Integer(n) => Value::Integer(Integer::from_bigint(n.clone())),
            ValueKey::Number(b) => Value::Number(f64::from_bits(*b)),
            ValueKey::String(s) => Value::String(s.clone()),
            ValueKey::Bool(b) => Value::Bool(*b),
            ValueKey::Nothing => Value::Nothing,
        }
    }

}

pub fn range_len(start: &Integer, stop: &Integer, step: &Integer) -> BigInt {
    let zero = Integer::Small(0);
    if step == &zero || (step > &zero && start >= stop) || (step < &zero && start <= stop) {
        return BigInt::from(0);
    }
    let diff = if step > &zero { stop.to_bigint() - start.to_bigint() } else { start.to_bigint() - stop.to_bigint() };
    let abs_step = if step > &zero { step.to_bigint() } else { -step.to_bigint() };
    (diff + &abs_step - 1) / abs_step
}

/// Resolve an index value against a length that may exceed `usize` (ranges
/// over arbitrary-precision integers). Negative indices count from the end.
pub fn bigint_index(value: &Value, len: &BigInt) -> Result<BigInt, String> {
    let n = match value {
        Value::Integer(n) => n.to_bigint(),
        Value::Number(n) if n.fract() == 0.0 => BigInt::from_f64(*n).unwrap_or_else(|| BigInt::from(0)),
        _ => return Err("Index must be integer".to_string()),
    };
    let i = if n < BigInt::from(0) { len + n } else { n };
    if i < BigInt::from(0) || i >= *len {
        Err("Index out of range".to_string())
    } else {
        Ok(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_equals_number() {
        assert_eq!(Value::integer(5), Value::Number(5.0));
        assert_ne!(Value::integer(5), Value::Number(5.5));
        // Large integers must not lose precision when compared with integral floats.
        assert_ne!(Value::big_integer(BigInt::from(i64::MAX)), Value::Number(i64::MAX as f64));
        assert_ne!(Value::big_integer(BigInt::from(9_007_199_254_740_993_i64)), Value::Number(9_007_199_254_740_992.0));
        assert_eq!(Value::big_integer(BigInt::from(9_007_199_254_740_992_i64)), Value::Number(9_007_199_254_740_992.0));
    }

    #[test]
    fn type_names() {
        assert_eq!(Value::integer(1).type_name(), "integer");
        assert_eq!(Value::Number(1.5).type_name(), "number");
        assert_eq!(Value::String("hi".to_string()).type_name(), "string");
        assert_eq!(Value::Bool(true).type_name(), "boolean");
    }

    #[test]
    fn list_display() {
        let list = Value::List(Rc::new(RefCell::new(vec![
            Value::integer(1),
            Value::integer(2),
        ])));
        assert_eq!(format!("{:?}", list), "[1, 2]");
    }

    #[test]
    fn range_len_calculations() {
        let i = |n: i64| Integer::from_i64(n);
        assert_eq!(range_len(&i(0), &i(10), &i(1)), BigInt::from(10));
        assert_eq!(range_len(&i(0), &i(10), &i(2)), BigInt::from(5));
        assert_eq!(range_len(&i(10), &i(0), &i(-2)), BigInt::from(5));
        assert_eq!(range_len(&i(0), &i(0), &i(1)), BigInt::from(0));
    }

    #[test]
    fn value_key_roundtrip() {
        let key = Value::integer(42).as_key().expect("integer should be a valid dict key");
        assert_eq!(key.to_value(), Value::integer(42));
    }
}

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use num_bigint::BigInt;
use num_traits::cast::{FromPrimitive, ToPrimitive};

use crate::ast::{BinOp, Span, UnaryOp};
use crate::bytecode::{CompiledFunction, Op};
use crate::environment::Environment;
use crate::interpreter::{Control, Interpreter};
use crate::value::{range_len, ErrorValue, Integer, VMClassValue, VMFunctionValue, Value};

pub struct Vm<'a> {
    interpreter: &'a mut Interpreter,
    stack: Vec<Value>,
    locals: Vec<Value>,
    frames: Vec<CallFrame>,
    try_stack: Vec<TryFrame>,
}

struct TryFrame {
    catch_ip: usize,
    catch_var_slot: usize,
    frame_depth: usize,
    stack_len: usize,
    locals_len: usize,
}

struct CallFrame {
    function: Rc<CompiledFunction>,
    closure: Rc<RefCell<Environment>>,
    upvalues: Vec<Rc<RefCell<Value>>>,
    ip: usize,
    slots_start: usize,
    pending_instance: Option<Value>,
}

impl<'a> Vm<'a> {
    pub(crate) fn stack_top(&self) -> Option<Value> {
        self.stack.last().cloned()
    }

    pub fn new(interpreter: &'a mut Interpreter, main: Rc<CompiledFunction>) -> Self {
        let closure = interpreter.env.clone();
        let mut locals = Vec::with_capacity(main.local_count);
        for _ in 0..main.local_count {
            locals.push(Value::Nothing);
        }
        Self {
            interpreter,
            stack: Vec::with_capacity(256),
            locals,
            frames: vec![CallFrame { function: main, closure, upvalues: Vec::new(), ip: 0, slots_start: 0, pending_instance: None }],
            try_stack: Vec::new(),
        }
    }

    pub fn run(&mut self) -> Result<(), Control> {
        loop {
            if self.frames.is_empty() {
                return Ok(());
            }
            let frame = self.frames.last_mut().unwrap();
            if frame.ip >= frame.function.chunk.ops.len() {
                // Should not happen because every path returns, but be safe.
                self.frames.pop();
                continue;
            }
            let ip = frame.ip;
            frame.ip += 1;
            let op_ptr: *const Op = &frame.function.chunk.ops[ip];
            let span_ptr: *const Span = &frame.function.chunk.spans[ip];
            // op/span are pointers into the current frame's chunk data, which is not
            // reallocated during execution. self.frames may reallocate on call/return,
            // but that does not move the chunk Vecs.
            let op = unsafe { &*op_ptr };
            let span = unsafe { &*span_ptr };

            match self.execute_op(op, span) {
                Ok(()) => {}
                Err(Control::RuntimeError(msg, span)) => {
                    self.handle_runtime_error(msg, span)?;
                    continue;
                }
                Err(other) => return Err(other),
            }
        }
    }

    #[inline(always)]
    fn execute_op(&mut self, op: &Op, span: &Span) -> Result<(), Control> {
        let frame = self.frames.last_mut().unwrap();
        match op {
            Op::Constant(idx) => {
                let value = frame.function.chunk.constants[*idx].clone();
                self.stack.push(value);
            }
            Op::Nothing => self.stack.push(Value::Nothing),
            Op::True => self.stack.push(Value::Bool(true)),
            Op::False => self.stack.push(Value::Bool(false)),
            Op::Pop => {
                self.stack.pop();
            }
            Op::Dup => {
                let v = self.stack.last().expect("stack underflow in dup").clone();
                self.stack.push(v);
            }
            Op::LoadLocal(slot) => {
                let value = match &self.locals[frame.slots_start + *slot] {
                    Value::Box(rc) => rc.borrow().clone(),
                    other => other.clone(),
                };
                if self.try_auto_call(&value, span)? {
                    // try_auto_call pushed a frame; return so the called function runs.
                    return Ok(());
                }
                self.stack.push(value);
            }
            Op::StoreLocal(slot) => {
                let value = self.stack.pop().expect("stack underflow in store local");
                match &mut self.locals[frame.slots_start + *slot] {
                    Value::Box(rc) => *rc.borrow_mut() = value,
                    slot_ref => *slot_ref = value,
                }
            }
            Op::GetUpvalue(slot) => {
                let value = frame.upvalues[*slot].borrow().clone();
                if self.try_auto_call(&value, span)? {
                    return Ok(());
                }
                self.stack.push(value);
            }
            Op::SetUpvalue(slot) => {
                let value = self.stack.pop().expect("stack underflow in set upvalue");
                *frame.upvalues[*slot].borrow_mut() = value;
            }
            Op::LoadGlobal(idx) => {
                let (_name, value) = {
                    let f = self.frames.last().unwrap();
                    let name = f.function.chunk.strings[*idx].clone();
                    let value = f.closure.borrow().get(&name).ok_or_else(|| {
                        Control::RuntimeError(format!("Undefined variable '{}'", name), span.clone())
                    })?;
                    (name, value)
                };
                if self.try_auto_call(&value, span)? {
                    return Ok(());
                }
                self.stack.push(value);
            }
            Op::StoreGlobal(idx) => {
                let name = {
                    let f = self.frames.last().unwrap();
                    f.function.chunk.strings[*idx].clone()
                };
                let value = self.stack.pop().expect("stack underflow in store global");
                let type_ann = {
                    let f = self.frames.last().unwrap();
                    f.closure.borrow().get_type(&name)
                };
                if let Some(Some(ann)) = type_ann {
                    self.check_type(&value, &ann, span)?;
                }
                self.frames.last().unwrap().closure.borrow().set(&name, value).map_err(Control::Error)?;
            }
            Op::DefineGlobal { name, type_ann } => {
                let (name, ann) = {
                    let f = self.frames.last().unwrap();
                    let name = f.function.chunk.strings[*name].clone();
                    let ann = (*type_ann).map(|i| f.function.chunk.strings[i].clone());
                    (name, ann)
                };
                let value = self.stack.pop().expect("stack underflow in define global");
                if let Some(ref a) = ann {
                    self.check_type(&value, a, span)?;
                }
                self.frames.last().unwrap().closure.borrow().define(&name, value, ann);
            }
            Op::Closure { func, upvalues } => {
                let (proto, closure, captured) = {
                    let f = self.frames.last_mut().unwrap();
                    let proto = Rc::clone(&f.function.chunk.functions[*func]);
                    let closure = f.closure.clone();
                    let mut captured = Vec::with_capacity(upvalues.len());
                    for uv in upvalues {
                        let slot_idx = f.slots_start + uv.index;
                        let rc = if uv.is_local {
                            match &mut self.locals[slot_idx] {
                                Value::Box(rc) => Rc::clone(rc),
                                slot => {
                                    let val = std::mem::replace(slot, Value::Nothing);
                                    let rc = Rc::new(RefCell::new(val));
                                    *slot = Value::Box(Rc::clone(&rc));
                                    rc
                                }
                            }
                        } else {
                            Rc::clone(&f.upvalues[uv.index])
                        };
                        captured.push(rc);
                    }
                    (proto, closure, captured)
                };
                let value = Value::VMFunction(Box::new(VMFunctionValue {
                    func: proto,
                    closure,
                    upvalues: captured,
                    from_module: self.interpreter.loading_module,
                }));
                self.stack.push(value);
            }
            Op::Binary(bin_op) => {
                let right = self.stack.pop().expect("stack underflow in binary");
                let left = self.stack.pop().expect("stack underflow in binary");
                let result = self.eval_binary(bin_op, left, right, span)?;
                self.stack.push(result);
            }
            Op::Unary(unary_op) => {
                let value = self.stack.pop().expect("stack underflow in unary");
                let result = match unary_op {
                    UnaryOp::Neg => self.eval_neg(value, span)?,
                    UnaryOp::Not => {
                        if let Value::Bool(b) = value {
                            Value::Bool(!b)
                        } else {
                            return Err(Control::RuntimeError(
                                "'not' requires a boolean operand".to_string(),
                                span.clone(),
                            ));
                        }
                    }
                };
                self.stack.push(result);
            }
            Op::Jump(target) => {
                frame.ip = *target;
            }
            Op::JumpIfFalse(target) => {
                let value = self.stack.pop().expect("stack underflow in jump if false");
                if !Interpreter::is_truthy(&value) {
                    frame.ip = *target;
                }
            }
            Op::JumpIfTrue(target) => {
                let value = self.stack.pop().expect("stack underflow in jump if true");
                if Interpreter::is_truthy(&value) {
                    frame.ip = *target;
                }
            }
            Op::Loop(target) => {
                frame.ip = *target;
            }
            Op::Call(arg_count) => {
                let mut args = Vec::with_capacity(*arg_count as usize);
                for _ in 0..*arg_count {
                    args.push(self.stack.pop().expect("stack underflow in call args"));
                }
                args.reverse();
                let callee = self.stack.pop().expect("stack underflow in callee");
                self.call_value(callee, args, span)?;
            }
            Op::Return => {
                let value = self.stack.pop().expect("stack underflow in return");
                // Check return type annotation.
                let func = self.frames.last().unwrap().function.clone();
                if let Some(ref ann) = func.return_type {
                    self.check_type(&value, ann, span)?;
                }
                let frame = self.frames.pop().unwrap();
                self.locals.truncate(frame.slots_start);
                if self.frames.is_empty() {
                    return Ok(());
                }
                let result = frame.pending_instance.unwrap_or(value);
                self.stack.push(result);
            }

            Op::Show => {
                let value = self.stack.pop().expect("stack underflow in show");
                let text = value.to_string();
                self.interpreter.output.push(text.clone());
                if !self.interpreter.silent {
                    println!("{}", text);
                }
            }
            Op::BuildList(count) => {
                let mut items = Vec::with_capacity(*count);
                for _ in 0..*count {
                    items.push(self.stack.pop().expect("stack underflow in list"));
                }
                items.reverse();
                self.stack.push(Value::List(Rc::new(RefCell::new(items))));
            }
            Op::BuildDict(count) => {
                let mut map = HashMap::with_capacity(*count);
                for _ in 0..*count {
                    let v = self.stack.pop().expect("stack underflow in dict value");
                    let k = self.stack.pop().expect("stack underflow in dict key");
                    let key = k.as_key().map_err(|m| Control::RuntimeError(m, span.clone()))?;
                    map.insert(key, v);
                }
                self.stack.push(Value::Dict(Rc::new(RefCell::new(map))));
            }
            Op::Index => {
                let index = self.stack.pop().expect("stack underflow in index");
                let object = self.stack.pop().expect("stack underflow in index object");
                let result = self.index_get(object, index, span)?;
                self.stack.push(result);
            }
            Op::IndexSet => {
                // Compiler pushes: value, object, index (top).
                let index = self.stack.pop().expect("stack underflow in index set");
                let object = self.stack.pop().expect("stack underflow in index set object");
                let value = self.stack.pop().expect("stack underflow in index set");
                self.index_set(object, index, value, span)?;
            }
            Op::PropertyGet(idx) => {
                let name = {
                    let f = self.frames.last().unwrap();
                    f.function.chunk.strings[*idx].clone()
                };
                let object = self.stack.pop().expect("stack underflow in property get");
                let result = self.property_get(object, &name, span)?;
                self.stack.push(result);
            }
            Op::PropertySet(idx) => {
                let name = {
                    let f = self.frames.last().unwrap();
                    f.function.chunk.strings[*idx].clone()
                };
                let object = self.stack.pop().expect("stack underflow in property set object");
                let value = self.stack.pop().expect("stack underflow in property set value");
                self.property_set(object, &name, value, span)?;
            }
            Op::New(arg_count) => {
                let mut args = Vec::with_capacity(*arg_count as usize);
                for _ in 0..*arg_count {
                    args.push(self.stack.pop().expect("stack underflow in new args"));
                }
                args.reverse();
                let class = self.stack.pop().expect("stack underflow in new class");
                let instance = self.new_instance(class, args, span)?;
                self.stack.push(instance);
            }
            Op::Tell { name: idx, arg_count } => {
                let name = {
                    let f = self.frames.last().unwrap();
                    f.function.chunk.strings[*idx].clone()
                };
                let mut args = Vec::with_capacity(*arg_count as usize);
                for _ in 0..*arg_count {
                    args.push(self.stack.pop().expect("stack underflow in tell args"));
                }
                args.reverse();
                let object = self.stack.pop().expect("stack underflow in tell object");
                self.call_method(object, &name, args, span)?;
            }
            Op::IterInit => {
                let value = self.stack.pop().expect("stack underflow in iter init");
                let items = match value {
                    Value::List(list) => list.borrow().clone(),
                    Value::String(s) => s.chars().map(|c| Value::String(c.to_string())).collect(),
                    Value::Dict(dict) => dict.borrow().keys().map(|k| k.to_value()).collect(),
                    _ => return Err(Control::RuntimeError(format!("Cannot iterate over {}", value.type_name()), span.clone())),
                };
                self.stack.push(Value::List(Rc::new(RefCell::new(items))));
            }
            Op::Length => {
                let value = self.stack.pop().expect("stack underflow in length");
                let len = match value {
                    Value::String(s) => s.len(),
                    Value::List(list) => list.borrow().len(),
                    Value::Dict(dict) => dict.borrow().len(),
                    Value::Range { start, stop, step } => range_len(start, stop, step) as usize,
                    _ => return Err(Control::RuntimeError(format!("Cannot get length of {}", value.type_name()), span.clone())),
                };
                self.stack.push(Value::integer(len as i64));
            }
            Op::TryBegin(catch_ip, catch_var_slot) => {
                self.try_stack.push(TryFrame {
                    catch_ip: *catch_ip,
                    catch_var_slot: *catch_var_slot,
                    frame_depth: self.frames.len(),
                    stack_len: self.stack.len(),
                    locals_len: self.locals.len(),
                });
            }
            Op::TryEnd => {
                self.try_stack.pop();
            }
            Op::Import(idx) => {
                let path = {
                    let f = self.frames.last().unwrap();
                    f.function.chunk.strings[*idx].clone()
                };
                let old_env = self.interpreter.env.clone();
                {
                    let f = self.frames.last().unwrap();
                    self.interpreter.env = f.closure.clone();
                }
                let result = self.interpreter.import_module(&path, span);
                self.interpreter.env = old_env;
                result?;
            }
            Op::QualifiedGet(module_idx, name_idx) => {
                let (module_name, name) = {
                    let f = self.frames.last().unwrap();
                    let module_name = f.function.chunk.strings[*module_idx].clone();
                    let name = f.function.chunk.strings[*name_idx].clone();
                    (module_name, name)
                };
                let value = {
                    let modules = self.interpreter.modules.borrow();
                    let mod_env = modules.get(&module_name).cloned().ok_or_else(|| {
                        Control::RuntimeError(format!("Module '{}' not imported", module_name), span.clone())
                    })?;
                    mod_env.borrow().get(&name).ok_or_else(|| {
                        Control::RuntimeError(format!("'{}' not found in module '{}'", name, module_name), span.clone())
                    })?
                };
                if self.try_auto_call(&value, span)? {
                    return Ok(());
                }
                self.stack.push(value);
            }
            Op::Export(indices) => {
                let names: Vec<String> = {
                    let f = self.frames.last().unwrap();
                    indices.iter().map(|i| f.function.chunk.strings[*i].clone()).collect()
                };
                let closure = self.frames.last().unwrap().closure.clone();
                for name in names {
                    closure.borrow().add_export(&name);
                }
            }
            Op::Read => {
                let path_value = self.stack.pop().expect("stack underflow in read");
                let path_str = match path_value {
                    Value::String(s) => s,
                    _ => return Err(Control::RuntimeError("Read path must be a string".to_string(), span.clone())),
                };
                let resolved = self.interpreter.resolve_path(&path_str);
                let content = std::fs::read_to_string(&resolved)
                    .map_err(|e| Control::Error(format!("Cannot read {}: {}", resolved.display(), e)))?;
                self.stack.push(Value::String(content));
            }
            Op::Write => {
                let path_value = self.stack.pop().expect("stack underflow in write path");
                let content_value = self.stack.pop().expect("stack underflow in write content");
                let path_str = match path_value {
                    Value::String(s) => s,
                    _ => return Err(Control::RuntimeError("Write path must be a string".to_string(), span.clone())),
                };
                let content_str = content_value.to_string();
                let resolved = self.interpreter.resolve_path(&path_str);
                std::fs::write(&resolved, content_str)
                    .map_err(|e| Control::Error(format!("Cannot write {}: {}", resolved.display(), e)))?;
            }
            Op::BuildClass { name, init, methods, fields, field_init } => {
                let f = self.frames.last().unwrap();
                let name_str = f.function.chunk.strings[*name].clone();
                let method_names: Vec<String> = methods.iter().map(|i| f.function.chunk.strings[*i].clone()).collect();
                let field_names: Vec<String> = fields.iter().map(|i| f.function.chunk.strings[*i].clone()).collect();
                let field_init: Vec<Option<usize>> = field_init.iter().map(|&i| if i == usize::MAX { None } else { Some(i) }).collect();
                let mut method_map: HashMap<String, Value> = HashMap::with_capacity(methods.len());
                for method_name in method_names.iter().rev() {
                    let func = self.stack.pop().expect("stack underflow in build class method");
                    method_map.insert(method_name.clone(), func);
                }
                let init_value = if init.is_some() {
                    Some(Box::new(self.stack.pop().expect("stack underflow in build class init")))
                } else {
                    None
                };
                let class = Value::VMClass(Box::new(VMClassValue {
                    name: name_str,
                    init: init_value,
                    methods: method_map,
                    field_names,
                    field_init,
                    from_module: self.interpreter.loading_module,
                }));
                self.stack.push(class);
            }
            Op::CheckType(idx) => {
                let ann = {
                    let f = self.frames.last().unwrap();
                    f.function.chunk.strings[*idx].clone()
                };
                let value = self.stack.last().expect("stack underflow in check type").clone();
                self.check_type(&value, &ann, span)?;
            }
            Op::IncrementLocal(slot) => {
                match &mut self.locals[frame.slots_start + *slot] {
                    Value::Box(rc) => {
                        let mut cell = rc.borrow_mut();
                        match &mut *cell {
                            Value::Integer(n) => n.add_assign(&Integer::from_i64(1)),
                            Value::Number(n) => *n += 1.0,
                            _ => return Err(Control::RuntimeError(
                                "Increment requires a number".to_string(), span.clone(),
                            )),
                        }
                    }
                    Value::Integer(n) => n.add_assign(&Integer::from_i64(1)),
                    Value::Number(n) => *n += 1.0,
                    _ => return Err(Control::RuntimeError(
                        "Increment requires a number".to_string(), span.clone(),
                    )),
                }
            }
            Op::AddLocals(target, source) => {
                if *target == *source {
                    let target_ref = &mut self.locals[frame.slots_start + *target];
                    match &mut *target_ref {
                        Value::Box(rc) => {
                            let mut cell = rc.borrow_mut();
                            match &mut *cell {
                                Value::Integer(n) => n.mul_assign(&Integer::from_i64(2)),
                                Value::Number(n) => *n *= 2.0,
                                _ => return Err(Control::RuntimeError(
                                    "Addition requires numbers".to_string(), span.clone(),
                                )),
                            }
                        }
                        Value::Integer(n) => n.mul_assign(&Integer::from_i64(2)),
                        Value::Number(n) => *n *= 2.0,
                        _ => return Err(Control::RuntimeError(
                            "Addition requires numbers".to_string(), span.clone(),
                        )),
                    }
                } else {
                    let source_val = match &self.locals[frame.slots_start + *source] {
                        Value::Box(rc) => rc.borrow().clone(),
                        other => other.clone(),
                    };
                    let target_ref = &mut self.locals[frame.slots_start + *target];
                    match &mut *target_ref {
                        Value::Box(rc) => {
                            let mut target_cell = rc.borrow_mut();
                            match &mut *target_cell {
                                Value::Integer(t) if matches!(source_val, Value::Integer(_)) => {
                                    if let Value::Integer(ref s) = source_val {
                                        t.add_assign(s);
                                    }
                                }
                                Value::Number(t) if matches!(source_val, Value::Number(_)) => {
                                    if let Value::Number(s) = source_val {
                                        *t += s;
                                    }
                                }
                                target_val => {
                                    let target_val = std::mem::replace(target_val, Value::Nothing);
                                    let result = Self::add_values(target_val, source_val, span)?;
                                    *target_cell = result;
                                }
                            }
                        }
                        Value::Integer(t) if matches!(source_val, Value::Integer(_)) => {
                            if let Value::Integer(ref s) = source_val {
                                t.add_assign(s);
                            }
                        }
                        Value::Number(t) if matches!(source_val, Value::Number(_)) => {
                            if let Value::Number(s) = source_val {
                                *t += s;
                            }
                        }
                        target_val => {
                            let target_val = std::mem::replace(target_val, Value::Nothing);
                            let result = Self::add_values(target_val, source_val, span)?;
                            *target_ref = result;
                        }
                    }
                }
            }
            Op::AppendLocalString { slot, string_idx } => {
                let s = frame.function.chunk.strings[*string_idx].clone();
                let target_ref = &mut self.locals[frame.slots_start + *slot];
                match target_ref {
                    Value::Box(rc) => {
                        let mut cell = rc.borrow_mut();
                        match &mut *cell {
                            Value::String(existing) => existing.push_str(&s),
                            other => {
                                let result = Self::add_values(std::mem::replace(other, Value::Nothing), Value::String(s), span)?;
                                *other = result;
                            }
                        }
                    }
                    Value::String(existing) => existing.push_str(&s),
                    other => {
                        let result = Self::add_values(std::mem::replace(other, Value::Nothing), Value::String(s), span)?;
                        *other = result;
                    }
                }
            }
            Op::AppendLocalList { slot } => {
                let item = self.stack.pop().expect("stack underflow in append local list");
                let target_ref = &mut self.locals[frame.slots_start + *slot];
                match target_ref {
                    Value::Box(rc) => {
                        let mut cell = rc.borrow_mut();
                        match &mut *cell {
                            Value::List(list) => {
                                if Rc::strong_count(list) > 1 {
                                    let mut new_items = list.borrow().clone();
                                    new_items.push(item);
                                    *cell = Value::List(Rc::new(RefCell::new(new_items)));
                                } else {
                                    list.borrow_mut().push(item);
                                }
                            }
                            other => {
                                let result = Self::add_values(std::mem::replace(other, Value::Nothing), Value::List(Rc::new(RefCell::new(vec![item]))), span)?;
                                *other = result;
                            }
                        }
                    }
                    Value::List(list) => {
                        if Rc::strong_count(list) > 1 {
                            let mut new_items = list.borrow().clone();
                            new_items.push(item);
                            *target_ref = Value::List(Rc::new(RefCell::new(new_items)));
                        } else {
                            list.borrow_mut().push(item);
                        }
                    }
                    other => {
                        let result = Self::add_values(std::mem::replace(other, Value::Nothing), Value::List(Rc::new(RefCell::new(vec![item]))), span)?;
                        *other = result;
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_runtime_error(&mut self, msg: String, span: Span) -> Result<(), Control> {
        let current_depth = self.frames.len();
        let target = self.try_stack.iter().rposition(|f| f.frame_depth <= current_depth);
        if let Some(idx) = target {
            let target_depth = self.try_stack[idx].frame_depth;
            // Discard any try frames that are inside the popped call frames.
            while self.try_stack.len() > idx + 1 {
                self.try_stack.pop();
            }
            let try_frame = self.try_stack.pop().unwrap();
            while self.frames.len() > target_depth {
                self.frames.pop();
            }
            self.stack.truncate(try_frame.stack_len);
            self.locals.truncate(try_frame.locals_len);
            let err = Value::Error(Box::new(ErrorValue {
                message: msg,
                line: span.line as i64,
                col: span.col as i64,
            }));
            if let Some(frame) = self.frames.last_mut() {
                self.locals[frame.slots_start + try_frame.catch_var_slot] = err;
                frame.ip = try_frame.catch_ip;
            }
            Ok(())
        } else {
            Err(Control::RuntimeError(msg, span))
        }
    }

    fn check_type(&mut self, value: &Value, ann: &str, span: &Span) -> Result<(), Control> {
        let interp: &Interpreter = self.interpreter;
        interp.check_type(value, ann, span)
    }

    fn try_auto_call(&mut self, value: &Value, span: &Span) -> Result<bool, Control> {
        match value {
            Value::BuiltIn(bv) if bv.min_arity == 0 && bv.max_arity == 0 => {
                let result = (bv.func)(&[]).map_err(|m| Control::RuntimeError(m, span.clone()))?;
                self.stack.push(result);
                Ok(true)
            }
            Value::Function(fv) if fv.params.is_empty() => {
                // Tree-walker function values are not expected in the VM path; fall back.
                Err(Control::RuntimeError(
                    "cannot auto-call tree-walker function in VM".to_string(),
                    span.clone(),
                ))
            }
            Value::VMFunction(fv) if fv.func.params.is_empty() => {
                self.call_value(value.clone(), Vec::new(), span)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(crate) fn call_value(&mut self, callee: Value, args: Vec<Value>, span: &Span) -> Result<(), Control> {
        match callee {
            Value::BuiltIn(bv) => {
                if args.len() < bv.min_arity || args.len() > bv.max_arity {
                    return Err(Control::RuntimeError("Wrong arity".to_string(), span.clone()));
                }
                let result = (bv.func)(&args).map_err(|m| Control::RuntimeError(m, span.clone()))?;
                self.stack.push(result);
                Ok(())
            }
            Value::VMFunction(fv) => {
                if fv.func.params.len() != args.len() {
                    return Err(Control::RuntimeError(
                        format!(
                            "Function {} expects {} args, got {}",
                            fv.func.name,
                            fv.func.params.len(),
                            args.len()
                        ),
                        fv.func.span.clone(),
                    ));
                }
                let slots_start = self.locals.len();
                self.locals.resize(slots_start + fv.func.local_count, Value::Nothing);
                for (i, arg) in args.into_iter().enumerate() {
                    self.locals[slots_start + i] = arg;
                }
                self.frames.push(CallFrame {
                    function: fv.func,
                    closure: fv.closure,
                    upvalues: fv.upvalues,
                    ip: 0,
                    slots_start,
                    pending_instance: None,
                });
                Ok(())
            }
            Value::Function(_) => {
                let closure = self.frames.last().unwrap().closure.clone();
                let old_env = self.interpreter.env.clone();
                self.interpreter.env = closure;
                let result = self.interpreter.call_value(&callee, args, span);
                self.interpreter.env = old_env;
                let value = result?;
                self.stack.push(value);
                Ok(())
            }
            Value::VMClass(_) => {
                let instance = self.new_instance(callee, args, span)?;
                self.stack.push(instance);
                Ok(())
            }
            Value::Class(_) => {
                let closure = self.frames.last().unwrap().closure.clone();
                let old_env = self.interpreter.env.clone();
                self.interpreter.env = closure;
                let result = self.interpreter.call_value(&callee, args, span);
                self.interpreter.env = old_env;
                let value = result?;
                self.stack.push(value);
                Ok(())
            }
            _ => Err(Control::RuntimeError(
                format!("Cannot call {}", callee.type_name()),
                span.clone(),
            )),
        }
    }

    fn add_values(left: Value, right: Value, span: &Span) -> Result<Value, Control> {
        match (&left, &right) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a.add(b))),
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            (Value::Integer(a), Value::Number(b)) => Ok(Value::Number(a.to_f64() + b)),
            (Value::Number(a), Value::Integer(b)) => Ok(Value::Number(a + b.to_f64())),
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
            (Value::List(a), Value::List(b)) => {
                let mut items = a.borrow().clone();
                items.extend(b.borrow().iter().cloned());
                Ok(Value::List(Rc::new(RefCell::new(items))))
            }
            _ => Err(Control::RuntimeError("Invalid operands for +".to_string(), span.clone())),
        }
    }

    fn eval_binary(&self, op: &BinOp, left: Value, right: Value, span: &Span) -> Result<Value, Control> {
        match op {
            BinOp::Add => Self::add_values(left, right, span),
            BinOp::Sub => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a.sub(b))),
                _ => self.numeric_op(&left, &right, |a, b| a - b, |a, b| a.to_f64() - b.to_f64(), span),
            },
            BinOp::Mul => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a.mul(b))),
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
                (Value::Integer(a), Value::Number(b)) => Ok(Value::Number(a.to_f64() * b)),
                (Value::Number(a), Value::Integer(b)) => Ok(Value::Number(a * b.to_f64())),
                (Value::String(s), Value::Integer(n)) | (Value::Integer(n), Value::String(s)) => {
                    if n.to_bigint() < BigInt::from(0) {
                        return Err(Control::RuntimeError("Cannot repeat string a negative number of times".to_string(), span.clone()));
                    }
                    let count = n.to_bigint().to_usize().ok_or_else(|| Control::RuntimeError("Cannot repeat string: count is too large".to_string(), span.clone()))?;
                    Ok(Value::String(s.repeat(count)))
                }
                _ => Err(Control::RuntimeError("Invalid operands for *".to_string(), span.clone())),
            },
            BinOp::Div => {
                if self.is_zero(&right) {
                    return Err(Control::RuntimeError("Division by zero.".to_string(), span.clone()));
                }
                self.numeric_op(&left, &right, |a, b| a / b, |a, b| a.to_f64() / b.to_f64(), span)
            }
            BinOp::Mod => {
                if self.is_zero(&right) {
                    return Err(Control::RuntimeError("Modulo by zero.".to_string(), span.clone()));
                }
                match (&left, &right) {
                    (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a.modulo(b).unwrap())),
                    _ => self.numeric_op(&left, &right, |a, b| a % b, |a, b| a.to_f64() % b.to_f64(), span),
                }
            }
            BinOp::Pow => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => {
                    match a.pow(b) {
                        Ok(result) => Ok(Value::Integer(result)),
                        Err("Division by zero") => return Err(Control::RuntimeError("Division by zero".to_string(), span.clone())),
                        Err(_) => return Err(Control::RuntimeError("Exponent too large".to_string(), span.clone())),
                    }
                }
                _ => self.numeric_op(&left, &right, |a, b| a.powf(b), |a, b| a.to_f64().powf(b.to_f64()), span),
            },
            BinOp::Eq => Ok(Value::Bool(left == right)),
            BinOp::Ne => Ok(Value::Bool(left != right)),
            BinOp::Lt => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Bool(a.cmp(b) == std::cmp::Ordering::Less)),
                _ => self.compare(&left, &right, |a, b| a < b, span),
            },
            BinOp::Gt => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Bool(a.cmp(b) == std::cmp::Ordering::Greater)),
                _ => self.compare(&left, &right, |a, b| a > b, span),
            },
            BinOp::Le => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Bool(a.cmp(b) != std::cmp::Ordering::Greater)),
                _ => self.compare(&left, &right, |a, b| a <= b, span),
            },
            BinOp::Ge => match (&left, &right) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Bool(a.cmp(b) != std::cmp::Ordering::Less)),
                _ => self.compare(&left, &right, |a, b| a >= b, span),
            },
            _ => unreachable!(),
        }
    }

    fn is_zero(&self, value: &Value) -> bool {
        match value {
            Value::Integer(n) => n.is_zero(),
            Value::Number(n) => *n == 0.0,
            _ => false,
        }
    }

    fn numeric_op<FN, FI>(&self, left: &Value, right: &Value, float_op: FN, int_to_float: FI, span: &Span) -> Result<Value, Control>
    where
        FN: Fn(f64, f64) -> f64,
        FI: Fn(&crate::value::Integer, &crate::value::Integer) -> f64,
    {
        match (left, right) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Number(int_to_float(a, b))),
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(float_op(*a, *b))),
            (Value::Integer(a), Value::Number(b)) => Ok(Value::Number(float_op(a.to_f64(), *b))),
            (Value::Number(a), Value::Integer(b)) => Ok(Value::Number(float_op(*a, b.to_f64()))),
            _ => Err(Control::RuntimeError("Operands must be numbers".to_string(), span.clone())),
        }
    }

    fn compare<F>(&self, left: &Value, right: &Value, op: F, span: &Span) -> Result<Value, Control>
    where
        F: Fn(f64, f64) -> bool,
    {
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
                a.cmp_integer_f64(*b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Value::Number(a), Value::Integer(b)) => {
                if !a.is_finite() {
                    return Err(Control::RuntimeError("Cannot compare these values".to_string(), span.clone()));
                }
                b.cmp_integer_f64(*a).map(|o| o.reverse()).unwrap_or(std::cmp::Ordering::Equal)
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
            Value::Integer(n) => Ok(Value::Integer(n.neg())),
            Value::Number(n) => Ok(Value::Number(-n)),
            _ => Err(Control::RuntimeError("Cannot negate this value".to_string(), span.clone())),
        }
    }

    fn as_index(&self, value: &Value, len: usize, span: &Span) -> Result<usize, Control> {
        let n = match value {
            Value::Integer(n) => n.to_bigint(),
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

    fn index_get(&self, obj: Value, idx: Value, span: &Span) -> Result<Value, Control> {
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
                Ok(Value::integer(start + step * (i as i64)))
            }
            _ => Err(Control::RuntimeError(format!("Cannot index into {}", obj.type_name()), span.clone())),
        }
    }

    fn index_set(&self, obj: Value, idx: Value, value: Value, span: &Span) -> Result<(), Control> {
        match obj {
            Value::List(list) => {
                let i = self.as_index(&idx, list.borrow().len(), span)?;
                list.borrow_mut()[i] = value;
                Ok(())
            }
            Value::Dict(dict) => {
                let key = idx.as_key().map_err(|m| Control::RuntimeError(m, span.clone()))?;
                dict.borrow_mut().insert(key, value);
                Ok(())
            }
            _ => Err(Control::RuntimeError(format!("Cannot index into {}", obj.type_name()), span.clone())),
        }
    }

    fn property_get(&self, obj: Value, name: &str, span: &Span) -> Result<Value, Control> {
        match obj {
            Value::Instance { ref class, ref fields, ref slots } => {
                if let Some(slots) = slots {
                    if let Some(idx) = match class.as_ref() {
                        Value::VMClass(cv) => cv.field_names.iter().position(|n| n == name),
                        _ => None,
                    } {
                        return Ok(slots.borrow().get(idx).cloned().unwrap_or(Value::Nothing));
                    }
                }
                if let Some(fields) = fields {
                    if let Some(v) = fields.borrow().get(name).cloned() {
                        return Ok(v);
                    }
                }
                match class.as_ref() {
                    Value::Class(cv) => {
                        if cv.methods.contains_key(name) {
                            return Err(Control::RuntimeError(
                                format!("method '{}' must be called with 'tell <object> to {}'", name, name),
                                span.clone(),
                            ));
                        }
                    }
                    Value::VMClass(cv) => {
                        if cv.methods.contains_key(name) {
                            return Err(Control::RuntimeError(
                                format!("method '{}' must be called with 'tell <object> to {}'", name, name),
                                span.clone(),
                            ));
                        }
                    }
                    _ => {}
                }
                Err(Control::RuntimeError(format!("'{}' has no property '{}'", obj.type_name(), name), span.clone()))
            }
            Value::Error(ev) => match name {
                "message" => Ok(Value::String(ev.message.clone())),
                "line" => Ok(Value::integer(ev.line)),
                "col" => Ok(Value::integer(ev.col)),
                _ => Err(Control::RuntimeError(format!("error has no property '{}'", name), span.clone())),
            },
            Value::Module(mv) => {
                mv.env.borrow().get(name).ok_or_else(|| {
                    Control::RuntimeError(format!("'{}' not found in module", name), span.clone())
                })
            }
            _ => Err(Control::RuntimeError(format!("Cannot get property '{}' from {}", name, obj.type_name()), span.clone())),
        }
    }

    fn property_set(&self, obj: Value, name: &str, value: Value, span: &Span) -> Result<(), Control> {
        match obj {
            Value::Instance { ref class, ref fields, ref slots } => {
                if let Some(slots) = slots {
                    if let Some(idx) = match class.as_ref() {
                        Value::VMClass(cv) => cv.field_names.iter().position(|n| n == name),
                        _ => None,
                    } {
                        slots.borrow_mut()[idx] = value;
                        return Ok(());
                    }
                }
                if let Some(fields) = fields {
                    fields.borrow_mut().insert(name.to_string(), value);
                    Ok(())
                } else {
                    Err(Control::RuntimeError(format!("Cannot set property '{}' on {}", name, obj.type_name()), span.clone()))
                }
            }
            _ => Err(Control::RuntimeError(format!("Cannot set property '{}' on {}", name, obj.type_name()), span.clone())),
        }
    }

    fn new_instance(&mut self, cls: Value, args: Vec<Value>, span: &Span) -> Result<Value, Control> {
        match cls {
            Value::VMClass(ref cv) => {
                let has_layout = !cv.field_names.is_empty();
                let slots = if has_layout {
                    Some(Rc::new(RefCell::new(vec![Value::Nothing; cv.field_names.len()])))
                } else {
                    None
                };
                let simple_init = has_layout && cv.field_init.iter().all(|m| m.is_some());
                let instance = Value::Instance {
                    class: Box::new(cls.clone()),
                    fields: if simple_init { None } else { Some(Rc::new(RefCell::new(HashMap::new()))) },
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
                    self.call_value((**init_value).clone(), init_args, span)?;
                    if let Some(frame) = self.frames.last_mut() {
                        frame.pending_instance = Some(instance.clone());
                    }
                } else if !args.is_empty() {
                    return Err(Control::RuntimeError("Class takes no init arguments".to_string(), span.clone()));
                }
                Ok(instance)
            }
            Value::Class(_) => {
                let closure = self.frames.last().unwrap().closure.clone();
                let old_env = self.interpreter.env.clone();
                self.interpreter.env = closure;
                let result = self.interpreter.call_value(&cls, args, span);
                self.interpreter.env = old_env;
                let instance = result?;
                Ok(instance)
            }
            _ => Err(Control::RuntimeError(format!("Cannot create instance of {}", cls.type_name()), span.clone())),
        }
    }

    fn call_method(&mut self, obj: Value, name: &str, args: Vec<Value>, span: &Span) -> Result<(), Control> {
        let class = match &obj {
            Value::Instance { class, .. } => class.clone(),
            _ => return Err(Control::RuntimeError(format!("Cannot send message to {}", obj.type_name()), span.clone())),
        };
        let method = match class.as_ref() {
            Value::VMClass(cv) => cv.methods.get(name).cloned(),
            Value::Class(cv) => cv.methods.get(name).cloned(),
            _ => None,
        };
        if let Some(method) = method {
            let mut method_args = vec![obj];
            method_args.extend(args);
            self.call_value(method, method_args, span)?;
            Ok(())
        } else {
            Err(Control::RuntimeError(format!("Cannot send message '{}' to {}", name, obj.type_name()), span.clone()))
        }
    }
}

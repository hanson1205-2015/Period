use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::ast::*;
use crate::bytecode::{CompiledFunction, Op, Upvalue};
use crate::value::Value;

#[derive(Debug, Clone)]
pub struct CompileError(pub String);

#[derive(Clone)]
struct Local {
    name: String,
    slot: usize,
    depth: usize,
    type_ann: Option<String>,
}

#[derive(Clone)]
struct CompilerState {
    function: CompiledFunction,
    locals: Vec<Local>,
    scope_depth: usize,
    upvalues: Vec<Upvalue>,
    captured_globals: HashSet<String>,
    parent: Option<Rc<RefCell<CompilerState>>>,
    temp_counter: usize,
}

impl CompilerState {
    fn new(
        name: impl Into<String>,
        params: Vec<(String, Option<String>)>,
        return_type: Option<String>,
        span: Span,
        captured_globals: HashSet<String>,
        parent: Option<Rc<RefCell<CompilerState>>>,
    ) -> Self {
        let mut state = Self {
            function: CompiledFunction::new(name, params.clone(), return_type, span),
            locals: Vec::new(),
            scope_depth: 0,
            upvalues: Vec::new(),
            captured_globals,
            parent,
            temp_counter: 0,
        };
        for (name, type_ann) in params {
            state.declare_local(&name, type_ann.clone());
        }
        state
    }

    fn resolve_upvalue(&mut self, name: &str) -> Option<usize> {
        let parent = self.parent.as_ref()?.clone();
        let local_slot = parent.borrow().resolve_local(name);
        if let Some(slot) = local_slot {
            return Some(self.add_upvalue(Upvalue { is_local: true, index: slot }));
        }
        let parent_has_parent = parent.borrow().parent.is_some();
        if parent_has_parent {
            if let Some(idx) = parent.borrow_mut().resolve_upvalue(name) {
                return Some(self.add_upvalue(Upvalue { is_local: false, index: idx }));
            }
        }
        None
    }

    fn declare_local(&mut self, name: &str, type_ann: Option<String>) -> usize {
        self.declare_local_at_depth(name, type_ann, self.scope_depth)
    }

    fn declare_local_at_depth(&mut self, name: &str, type_ann: Option<String>, depth: usize) -> usize {
        let slot = self.locals.len();
        self.locals.push(Local {
            name: name.to_string(),
            slot,
            depth,
            type_ann,
        });
        self.function.local_count = self.function.local_count.max(slot + 1);
        slot
    }

    fn declare_temp_local(&mut self) -> usize {
        let name = format!("__logic_tmp_{}", self.temp_counter);
        self.temp_counter += 1;
        self.declare_local_at_depth(&name, None, self.scope_depth)
    }

    fn resolve_local(&self, name: &str) -> Option<usize> {
        for local in self.locals.iter().rev() {
            if local.name == name {
                return Some(local.slot);
            }
        }
        None
    }

    fn add_upvalue(&mut self, upvalue: Upvalue) -> usize {
        if let Some(idx) = self.upvalues.iter().position(|u| u.is_local == upvalue.is_local && u.index == upvalue.index) {
            return idx;
        }
        let idx = self.upvalues.len();
        self.upvalues.push(upvalue);
        idx
    }
}

pub struct Compiler {
    state: Rc<RefCell<CompilerState>>,
    enclosing: Option<Rc<RefCell<CompilerState>>>,
}

#[derive(Clone, Copy)]
enum VariableRef {
    Local(usize),
    Upvalue(usize),
    Global,
}

fn collect_class_fields(init: &Option<Init>) -> Vec<String> {
    let mut fields = Vec::new();
    if let Some(init) = init {
        collect_this_fields_in_stmts(&init.body, &mut fields);
    }
    fields
}

fn collect_this_fields_in_stmts(stmts: &[Stmt], fields: &mut Vec<String>) {
    for stmt in stmts {
        collect_this_fields_in_stmt(stmt, fields);
    }
}

fn collect_this_fields_in_stmt(stmt: &Stmt, fields: &mut Vec<String>) {
    match stmt {
        Stmt::Set { target, value, .. } => {
            if let AssignTarget::Property { object, name, .. } = target {
                if is_this_var(object) && !fields.contains(name) {
                    fields.push(name.clone());
                }
            }
            collect_this_fields_in_expr(value, fields);
        }
        Stmt::Let { value, .. }
        | Stmt::Show(value)
        | Stmt::Return { value: Some(value), .. }
        | Stmt::Expr(value) => collect_this_fields_in_expr(value, fields),
        Stmt::If { cond, then_branch, else_branch } => {
            collect_this_fields_in_expr(cond, fields);
            collect_this_fields_in_stmts(then_branch, fields);
            collect_this_fields_in_stmts(else_branch, fields);
        }
        Stmt::While { cond, body } => {
            collect_this_fields_in_expr(cond, fields);
            collect_this_fields_in_stmts(body, fields);
        }
        Stmt::For { iterable, body, .. } => {
            collect_this_fields_in_expr(iterable, fields);
            collect_this_fields_in_stmts(body, fields);
        }
        Stmt::Try { body, catch_body, .. } => {
            collect_this_fields_in_stmts(body, fields);
            collect_this_fields_in_stmts(catch_body, fields);
        }
        Stmt::Define { body, .. } => collect_this_fields_in_stmts(body, fields),
        Stmt::Class { init, methods, .. } => {
            if let Some(init) = init {
                collect_this_fields_in_stmts(&init.body, fields);
            }
            for m in methods { collect_this_fields_in_stmt(m, fields); }
        }
        Stmt::Read { path, .. }
        | Stmt::Write { path, .. } => collect_this_fields_in_expr(path, fields),
        Stmt::Init(init) => collect_this_fields_in_stmts(&init.body, fields),
        _ => {}
    }
}

fn collect_this_fields_in_expr(expr: &Expr, fields: &mut Vec<String>) {
    match expr {
        Expr::Property { object, name, .. } => {
            collect_this_fields_in_expr(object, fields);
            if is_this_var(object) && !fields.contains(name) {
                fields.push(name.clone());
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_this_fields_in_expr(left, fields);
            collect_this_fields_in_expr(right, fields);
        }
        Expr::Unary { operand, .. } => collect_this_fields_in_expr(operand, fields),
        Expr::Call { callee, args, .. } => {
            collect_this_fields_in_expr(callee, fields);
            for a in args { collect_this_fields_in_expr(a, fields); }
        }
        Expr::Index { object, index, .. } => {
            collect_this_fields_in_expr(object, fields);
            collect_this_fields_in_expr(index, fields);
        }
        Expr::New { class, args, .. } => {
            collect_this_fields_in_expr(class, fields);
            for a in args { collect_this_fields_in_expr(a, fields); }
        }
        Expr::Tell { object, args, .. } => {
            collect_this_fields_in_expr(object, fields);
            for a in args { collect_this_fields_in_expr(a, fields); }
        }
        Expr::List(items, _) => {
            for item in items { collect_this_fields_in_expr(item, fields); }
        }
        Expr::Dict(entries, _) => {
            for (k, v) in entries {
                collect_this_fields_in_expr(k, fields);
                collect_this_fields_in_expr(v, fields);
            }
        }
        _ => {}
    }
}

fn is_this_var(expr: &Expr) -> bool {
    matches!(expr, Expr::Variable { name, .. } if name == "this")
}

fn analyze_class_init(init: &Option<Init>, field_names: &[String]) -> Option<Vec<Option<usize>>> {
    let init = init.as_ref()?;
    let mut mapping = vec![None; field_names.len()];
    for stmt in &init.body {
        if let Stmt::Set { target, value, .. } = stmt {
            if let AssignTarget::Property { object, name, .. } = target {
                if is_this_var(object) {
                    if let Expr::Variable { name: param_name, .. } = value {
                        let field_idx = field_names.iter().position(|n| n == name)?;
                        let param_idx = init.params.iter().position(|(p, _)| p == param_name)?;
                        mapping[field_idx] = Some(param_idx);
                        continue;
                    }
                }
            }
        }
        // Any other statement makes the init too complex for direct field copying.
        return None;
    }
    Some(mapping)
}

impl Compiler {
    pub fn new(
        name: impl Into<String>,
        params: Vec<(String, Option<String>)>,
        return_type: Option<String>,
        span: Span,
        captured_globals: HashSet<String>,
    ) -> Self {
        Self {
            state: Rc::new(RefCell::new(CompilerState::new(
                name, params, return_type, span, captured_globals, None,
            ))),
            enclosing: None,
        }
    }

    fn new_child(
        name: impl Into<String>,
        params: Vec<(String, Option<String>)>,
        return_type: Option<String>,
        span: Span,
        enclosing: Rc<RefCell<CompilerState>>,
    ) -> Self {
        Self {
            state: Rc::new(RefCell::new(CompilerState::new(
                name, params, return_type, span, HashSet::new(), Some(enclosing),
            ))),
            enclosing: None,
        }
    }

    pub fn compile_program(stmts: &[Stmt]) -> Result<CompiledFunction, CompileError> {
        let mut stmts: Vec<Stmt> = stmts.to_vec();
        crate::inline::inline_small_functions(&mut stmts);
        let captured_globals = collect_captured_globals(&stmts);
        let compiler = Compiler::new("<main>", Vec::new(), None, Span { line: 1, col: 1 }, captured_globals);
        for stmt in &stmts {
            compiler.compile_stmt(stmt)?;
        }
        compiler.emit(Op::Nothing, Span { line: 1, col: 1 });
        compiler.emit(Op::Return, Span { line: 1, col: 1 });
        Ok(compiler.finish())
    }

    fn finish(&self) -> CompiledFunction {
        let mut func = self.state.borrow().function.clone();
        func.upvalues = self.state.borrow().upvalues.clone();
        func
    }

    fn state(&self) -> std::cell::Ref<'_, CompilerState> {
        self.state.borrow()
    }

    fn state_mut(&self) -> std::cell::RefMut<'_, CompilerState> {
        self.state.borrow_mut()
    }

    fn temp_local(&self) -> usize {
        self.state.borrow_mut().declare_temp_local()
    }

    fn emit(&self, op: Op, span: Span) -> usize {
        self.state_mut().function.chunk.emit(op, span)
    }

    fn add_constant(&self, value: Value) -> usize {
        self.state_mut().function.chunk.add_constant(value)
    }

    fn add_string(&self, s: &str) -> usize {
        let mut state = self.state_mut();
        if let Some(idx) = state.function.chunk.strings.iter().position(|x| x == s) {
            return idx;
        }
        state.function.chunk.add_string(s.to_string())
    }

    fn add_function(&self, func: CompiledFunction) -> usize {
        self.state_mut().function.chunk.add_function(func)
    }

    fn resolve_variable(&self, name: &str) -> VariableRef {
        if let Some(slot) = self.state().resolve_local(name) {
            return VariableRef::Local(slot);
        }
        if let Some(idx) = self.state_mut().resolve_upvalue(name) {
            return VariableRef::Upvalue(idx);
        }
        VariableRef::Global
    }

    fn compile_stmt(&self, stmt: &Stmt) -> Result<(), CompileError> {
        match stmt {
            Stmt::Let { name, type_ann, value, span } => {
                self.compile_expr(value)?;
                if let Some(ann) = type_ann {
                    let idx = self.add_string(ann);
                    self.emit(Op::CheckType(idx), span.clone());
                }
                let (scope_depth, captured_globals) = {
                    let state = self.state();
                    (state.scope_depth, state.captured_globals.clone())
                };
                if scope_depth == 0 && captured_globals.contains(name) {
                    let slot = if let Some(slot) = self.state().resolve_local(name) {
                        slot
                    } else {
                        self.declare_local(name, type_ann.clone())
                    };
                    let name_idx = self.add_string(name);
                    let type_ann_idx = type_ann.as_ref().map(|a| self.add_string(a));
                    self.emit(Op::Dup, span.clone());
                    self.emit(Op::DefineGlobal { name: name_idx, type_ann: type_ann_idx }, span.clone());
                    self.emit(Op::StoreLocal(slot), span.clone());
                } else {
                    let existing = {
                        let state = self.state();
                        state.resolve_local(name).map(|s| (s, state.locals[s].depth))
                    };
                    if let Some((slot, depth)) = existing {
                        if depth == scope_depth {
                            self.emit(Op::StoreLocal(slot), span.clone());
                        } else {
                            let slot = self.declare_local(name, type_ann.clone());
                            self.emit(Op::StoreLocal(slot), span.clone());
                        }
                    } else {
                        let slot = self.declare_local(name, type_ann.clone());
                        self.emit(Op::StoreLocal(slot), span.clone());
                    }
                }
            }
            Stmt::Set { target, value } => {
                match target {
                    AssignTarget::Variable { name, span } => {
                        match self.resolve_variable(name) {
                            VariableRef::Local(slot) => {
                                let type_ann = self.state().locals[slot].type_ann.clone();
                                let local_name = self.state().locals[slot].name.clone();
                                let mut append_op: Option<Op> = None;
                                if let Expr::Binary { op: BinOp::Add, left, right, .. } = value {
                                    if Self::is_var_named(left, &local_name) {
                                        match right.as_ref() {
                                            Expr::String(s, _) if type_ann.as_deref().map_or(true, |a| a == "string") => {
                                                let string_idx = self.add_string(s);
                                                append_op = Some(Op::AppendLocalString { slot, string_idx });
                                            }
                                            Expr::List(items, _) if items.len() == 1 && type_ann.as_deref().map_or(true, |a| a == "list") => {
                                                self.compile_expr(&items[0])?;
                                                append_op = Some(Op::AppendLocalList { slot });
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                if let Some(op) = append_op {
                                    self.emit(op, span.clone());
                                } else if let Some(op) = self.try_peephole_set(slot, value) {
                                    self.emit(op, span.clone());
                                } else {
                                    self.compile_expr(value)?;
                                    if let Some(ann) = type_ann {
                                        let idx = self.add_string(&ann);
                                        self.emit(Op::CheckType(idx), span.clone());
                                    }
                                    self.emit(Op::StoreLocal(slot), span.clone());
                                }
                            }
                            VariableRef::Upvalue(slot) => {
                                self.compile_expr(value)?;
                                self.emit(Op::SetUpvalue(slot), span.clone());
                            }
                            VariableRef::Global => {
                                self.compile_expr(value)?;
                                let idx = self.add_string(name);
                                self.emit(Op::StoreGlobal(idx), span.clone());
                            }
                        }
                    }
                    AssignTarget::Index { object, index, span } => {
                        self.compile_expr(value)?;
                        self.compile_expr(object)?;
                        self.compile_expr(index)?;
                        self.emit(Op::IndexSet, span.clone());
                    }
                    AssignTarget::Property { object, name, span } => {
                        self.compile_expr(value)?;
                        self.compile_expr(object)?;
                        let idx = self.add_string(name);
                        self.emit(Op::PropertySet(idx), span.clone());
                    }
                }
            }
            Stmt::Show(expr) => {
                let span = expr.span().cloned().unwrap_or_else(|| Span { line: 1, col: 1 });
                self.compile_expr(expr)?;
                self.emit(Op::Show, span);
            }
            Stmt::If { cond, then_branch, else_branch } => {
                let span = cond.span().cloned().unwrap_or_else(|| Span { line: 1, col: 1 });
                self.compile_expr(cond)?;
                let then_jump = self.emit(Op::JumpIfFalse(0), span.clone());
                self.enter_scope();
                for stmt in then_branch {
                    self.compile_stmt(stmt)?;
                }
                self.leave_scope();
                let else_jump = if !else_branch.is_empty() {
                    Some(self.emit(Op::Jump(0), span.clone()))
                } else {
                    None
                };
                self.patch_jump(then_jump);
                if !else_branch.is_empty() {
                    self.enter_scope();
                    for stmt in else_branch {
                        self.compile_stmt(stmt)?;
                    }
                    self.leave_scope();
                    if let Some(j) = else_jump {
                        self.patch_jump(j);
                    }
                }
            }
            Stmt::While { cond, body } => {
                let span = cond.span().cloned().unwrap_or_else(|| Span { line: 1, col: 1 });
                let loop_start = self.state().function.chunk.ops.len();
                self.compile_expr(cond)?;
                let exit_jump = self.emit(Op::JumpIfFalse(0), span.clone());
                self.enter_scope();
                for stmt in body {
                    self.compile_stmt(stmt)?;
                }
                self.leave_scope();
                self.emit(Op::Loop(loop_start), span.clone());
                self.patch_jump(exit_jump);
            }
            Stmt::For { var, iterable, body } => {
                let range = self.try_compile_range(iterable)?;
                if let Some((start, stop, step)) = range {
                    self.compile_for_range(var, start, stop, step, body)?;
                } else {
                    self.compile_for_iter(var, iterable, body)?;
                }
            }
            Stmt::Return { value, span } => {
                if let Some(expr) = value {
                    self.compile_expr(expr)?;
                } else {
                    self.emit(Op::Nothing, span.clone());
                }
                self.emit(Op::Return, span.clone());
            }
            Stmt::Try { body, catch_var, catch_body } => {
                let span = Span { line: 1, col: 1 };
                let catch_slot = self.declare_local(catch_var, None);
                let try_begin = self.emit(Op::TryBegin(0, catch_slot), span.clone());
                self.enter_scope();
                for stmt in body {
                    self.compile_stmt(stmt)?;
                }
                self.leave_scope();
                self.emit(Op::TryEnd, span.clone());
                let after_catch = self.emit(Op::Jump(0), span.clone());
                self.patch_try_begin(try_begin);
                self.enter_scope();
                for stmt in catch_body {
                    self.compile_stmt(stmt)?;
                }
                self.leave_scope();
                self.patch_jump(after_catch);
            }
            Stmt::Define { name, params, return_type, body, span, .. } => {
                // Declare the function name as a local *before* compiling the body so
                // recursive calls resolve to the correct slot.
                let name_slot = self.declare_local(name, None);
                let mut func_compiler = Compiler::new_child(
                    name.clone(),
                    params.clone(),
                    return_type.clone(),
                    span.clone(),
                    self.state.clone(),
                );
                for stmt in body {
                    func_compiler.compile_stmt(stmt)?;
                }
                let end_span = Span { line: span.line, col: span.col };
                func_compiler.emit(Op::Nothing, end_span.clone());
                func_compiler.emit(Op::Return, end_span);
                let compiled = func_compiler.finish();
                let upvalues = compiled.upvalues.clone();
                let func_idx = self.add_function(compiled);
                self.emit(Op::Closure { func: func_idx, upvalues }, span.clone());
                self.emit(Op::StoreLocal(name_slot), span.clone());
            }
            Stmt::Class { name, init, methods, span, .. } => {
                let init_idx = if let Some(init) = init {
                    let mut init_params = vec![("this".to_string(), None)];
                    init_params.extend(init.params.clone());
                    let mut init_compiler = Compiler::new_child(
                        "<init>",
                        init_params,
                        None,
                        span.clone(),
                        self.state.clone(),
                    );
                    for stmt in &init.body {
                        init_compiler.compile_stmt(stmt)?;
                    }
                    let end_span = Span { line: span.line, col: span.col };
                    init_compiler.emit(Op::Nothing, end_span.clone());
                    init_compiler.emit(Op::Return, end_span);
                    let compiled = init_compiler.finish();
                    let upvalues = compiled.upvalues.clone();
                    let idx = self.add_function(compiled);
                    self.emit(Op::Closure { func: idx, upvalues }, span.clone());
                    Some(idx)
                } else {
                    None
                };

                let mut method_name_indices = Vec::new();
                for m in methods {
                    if let Stmt::Define { name: mname, params, return_type, body, span: mspan, .. } = m {
                        let mut method_params = vec![("this".to_string(), None)];
                        method_params.extend(params.clone());
                        let mut method_compiler = Compiler::new_child(
                            mname.clone(),
                            method_params,
                            return_type.clone(),
                            mspan.clone(),
                            self.state.clone(),
                        );
                        for stmt in body {
                            method_compiler.compile_stmt(stmt)?;
                        }
                        let end_span = Span { line: mspan.line, col: mspan.col };
                        method_compiler.emit(Op::Nothing, end_span.clone());
                        method_compiler.emit(Op::Return, end_span);
                        let compiled = method_compiler.finish();
                        let upvalues = compiled.upvalues.clone();
                        let idx = self.add_function(compiled);
                        self.emit(Op::Closure { func: idx, upvalues }, mspan.clone());
                        method_name_indices.push(self.add_string(mname));
                    }
                }

                let field_names = collect_class_fields(init);
                let field_indices: Vec<usize> = field_names.iter().map(|n| self.add_string(n)).collect();
                let field_init = analyze_class_init(init, &field_names)
                    .map(|m| m.into_iter().map(|o| o.unwrap_or(usize::MAX)).collect())
                    .unwrap_or_default();
                let name_idx = self.add_string(name);
                self.emit(Op::BuildClass { name: name_idx, init: init_idx, methods: method_name_indices, fields: field_indices, field_init }, span.clone());
                if self.state().scope_depth == 0 {
                    self.emit(Op::DefineGlobal { name: name_idx, type_ann: None }, span.clone());
                } else {
                    let slot = self.declare_local(name, None);
                    self.emit(Op::StoreLocal(slot), span.clone());
                }
            }
            Stmt::Import(paths) => {
                for (path, _) in paths {
                    let idx = self.add_string(path);
                    self.emit(Op::Import(idx), Span { line: 1, col: 1 });
                }
            }
            Stmt::Export(names) => {
                let indices: Vec<usize> = names.iter().map(|n| self.add_string(n)).collect();
                self.emit(Op::Export(indices), Span { line: 1, col: 1 });
            }
            Stmt::Read { name, path } => {
                let span = path.span().cloned().unwrap_or_else(|| Span { line: 1, col: 1 });
                self.compile_expr(path)?;
                self.emit(Op::Read, span.clone());
                if self.state().scope_depth == 0 {
                    let name_idx = self.add_string(name);
                    self.emit(Op::DefineGlobal { name: name_idx, type_ann: None }, span.clone());
                } else {
                    let slot = self.declare_local(name, None);
                    self.emit(Op::StoreLocal(slot), span.clone());
                }
            }
            Stmt::Write { content, path } => {
                let span = path.span().cloned().unwrap_or_else(|| Span { line: 1, col: 1 });
                self.compile_expr(content)?;
                self.compile_expr(path)?;
                self.emit(Op::Write, span);
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr)?;
                let span = expr.span().cloned().unwrap_or_else(|| Span { line: 1, col: 1 });
                self.emit(Op::Pop, span);
            }
            Stmt::Pass => {}
            _ => {
                return Err(CompileError("statement not supported by VM".to_string()));
            }
        }
        Ok(())
    }

    fn enter_scope(&self) {
        self.state_mut().scope_depth += 1;
    }

    fn leave_scope(&self) {
        self.state_mut().scope_depth -= 1;
        self.trim_locals();
    }

    fn trim_locals(&self) {
        let scope_depth = self.state().scope_depth;
        loop {
            let should_pop = self.state().locals.last().map(|l| l.depth > scope_depth).unwrap_or(false);
            if !should_pop {
                break;
            }
            self.state_mut().locals.pop();
        }
    }

    fn patch_jump(&self, idx: usize) {
        self.state_mut().function.chunk.patch_jump(idx);
    }

    fn patch_try_begin(&self, idx: usize) {
        self.state_mut().function.chunk.patch_try_begin(idx);
    }

    fn declare_local(&self, name: &str, type_ann: Option<String>) -> usize {
        self.state_mut().declare_local(name, type_ann)
    }

    fn compile_for_range(
        &self,
        var: &str,
        start: Option<Expr>,
        stop: Option<Expr>,
        step: Option<Expr>,
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        let span = Span { line: 1, col: 1 };
        let zero_idx = self.add_constant(Value::integer(0));
        let has_stop = stop.is_some();
        let stop_expr = if has_stop { stop.unwrap() } else { start.as_ref().unwrap().clone() };
        let start_expr: Expr = if has_stop { start.unwrap() } else { Expr::Integer(num_bigint::BigInt::from(0), span.clone()) };
        let step_expr: Expr = if let Some(s) = step { s } else { Expr::Integer(num_bigint::BigInt::from(1), span.clone()) };

        self.compile_expr(&start_expr)?;
        self.compile_expr(&stop_expr)?;
        self.compile_expr(&step_expr)?;

        let loop_depth = self.state().scope_depth + 1;
        let step_slot = self.declare_local_at_depth("__step", None, loop_depth);
        let stop_slot = self.declare_local_at_depth("__stop", None, loop_depth);
        let loop_slot = self.declare_local_at_depth(var, None, loop_depth);

        self.emit(Op::StoreLocal(step_slot), span.clone());
        self.emit(Op::StoreLocal(stop_slot), span.clone());
        self.emit(Op::StoreLocal(loop_slot), span.clone());

        let cond_start = self.state().function.chunk.ops.len();

        self.emit(Op::LoadLocal(step_slot), span.clone());
        self.emit(Op::Constant(zero_idx), span.clone());
        self.emit(Op::Binary(BinOp::Gt), span.clone());
        let pos_check = self.emit(Op::JumpIfFalse(0), span.clone());

        self.emit(Op::LoadLocal(loop_slot), span.clone());
        self.emit(Op::LoadLocal(stop_slot), span.clone());
        self.emit(Op::Binary(BinOp::Lt), span.clone());
        let to_body = self.emit(Op::Jump(0), span.clone());

        self.patch_jump(pos_check);
        self.emit(Op::LoadLocal(loop_slot), span.clone());
        self.emit(Op::LoadLocal(stop_slot), span.clone());
        self.emit(Op::Binary(BinOp::Gt), span.clone());

        self.patch_jump(to_body);
        let exit_jump = self.emit(Op::JumpIfFalse(0), span.clone());

        self.enter_scope();
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.leave_scope();

        self.emit(Op::LoadLocal(loop_slot), span.clone());
        self.emit(Op::LoadLocal(step_slot), span.clone());
        self.emit(Op::Binary(BinOp::Add), span.clone());
        self.emit(Op::StoreLocal(loop_slot), span.clone());
        self.emit(Op::Loop(cond_start), span.clone());

        self.patch_jump(exit_jump);
        Ok(())
    }

    fn compile_for_iter(&self, var: &str, iterable: &Expr, body: &[Stmt]) -> Result<(), CompileError> {
        let span = iterable.span().cloned().unwrap_or_else(|| Span { line: 1, col: 1 });
        self.compile_expr(iterable)?;
        self.emit(Op::IterInit, span.clone());

        let loop_depth = self.state().scope_depth + 1;
        let items_slot = self.declare_local_at_depth("__items", None, loop_depth);
        let idx_slot = self.declare_local_at_depth("__idx", None, loop_depth);
        let var_slot = self.declare_local_at_depth(var, None, loop_depth);

        self.emit(Op::StoreLocal(items_slot), span.clone());

        let zero_idx = self.add_constant(Value::integer(0));
        self.emit(Op::Constant(zero_idx), span.clone());
        self.emit(Op::StoreLocal(idx_slot), span.clone());

        let cond_start = self.state().function.chunk.ops.len();
        self.emit(Op::LoadLocal(idx_slot), span.clone());
        self.emit(Op::LoadLocal(items_slot), span.clone());
        self.emit(Op::Length, span.clone());
        self.emit(Op::Binary(BinOp::Lt), span.clone());
        let exit_jump = self.emit(Op::JumpIfFalse(0), span.clone());

        self.emit(Op::LoadLocal(items_slot), span.clone());
        self.emit(Op::LoadLocal(idx_slot), span.clone());
        self.emit(Op::Index, span.clone());
        self.emit(Op::StoreLocal(var_slot), span.clone());

        self.enter_scope();
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.leave_scope();

        let one_idx = self.add_constant(Value::integer(1));
        self.emit(Op::LoadLocal(idx_slot), span.clone());
        self.emit(Op::Constant(one_idx), span.clone());
        self.emit(Op::Binary(BinOp::Add), span.clone());
        self.emit(Op::StoreLocal(idx_slot), span.clone());
        self.emit(Op::Loop(cond_start), span.clone());

        self.patch_jump(exit_jump);
        Ok(())
    }

    fn compile_expr(&self, expr: &Expr) -> Result<(), CompileError> {
        let span = expr.span().cloned().unwrap_or_else(|| Span { line: 1, col: 1 });
        match expr {
            Expr::Integer(n, _) => {
                let idx = self.add_constant(Value::big_integer(n.clone()));
                self.emit(Op::Constant(idx), span);
            }
            Expr::Number(n, _) => {
                let idx = self.add_constant(Value::Number(*n));
                self.emit(Op::Constant(idx), span);
            }
            Expr::String(s, _) => {
                let idx = self.add_constant(Value::String(s.clone()));
                self.emit(Op::Constant(idx), span);
            }
            Expr::Bool(b, _) => {
                self.emit(if *b { Op::True } else { Op::False }, span);
            }
            Expr::Nothing(_) | Expr::Ellipsis => {
                self.emit(Op::Nothing, span);
            }
            Expr::Variable { name, span } => {
                match self.resolve_variable(name) {
                    VariableRef::Local(slot) => { self.emit(Op::LoadLocal(slot), span.clone()); }
                    VariableRef::Upvalue(slot) => { self.emit(Op::GetUpvalue(slot), span.clone()); }
                    VariableRef::Global => {
                        let idx = self.add_string(name);
                        self.emit(Op::LoadGlobal(idx), span.clone());
                    }
                }
            }
            Expr::Binary { op, left, right, span } => {
                if *op == BinOp::And || *op == BinOp::Or {
                    // Use a temporary local so control-flow joins do not need to
                    // pass a value on the evaluation stack across basic blocks.
                    let tmp = self.temp_local();
                    self.compile_expr(left)?;
                    self.emit(Op::StoreLocal(tmp), span.clone());
                    self.emit(Op::LoadLocal(tmp), span.clone());
                    let short_jump = if *op == BinOp::And {
                        self.emit(Op::JumpIfFalse(0), span.clone())
                    } else {
                        self.emit(Op::JumpIfTrue(0), span.clone())
                    };
                    self.compile_expr(right)?;
                    self.emit(Op::StoreLocal(tmp), span.clone());
                    self.patch_jump(short_jump);
                    self.emit(Op::LoadLocal(tmp), span.clone());
                } else {
                    self.compile_expr(left)?;
                    self.compile_expr(right)?;
                    self.emit(Op::Binary(*op), span.clone());
                }
            }
            Expr::Unary { op, operand, span } => {
                self.compile_expr(operand)?;
                self.emit(Op::Unary(*op), span.clone());
            }
            Expr::Call { callee, args, span } => {
                self.compile_expr(callee)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.emit(Op::Call(args.len() as u8), span.clone());
            }
            Expr::Index { object, index, span } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.emit(Op::Index, span.clone());
            }
            Expr::Property { object, name, span } => {
                self.compile_expr(object)?;
                let idx = self.add_string(name);
                self.emit(Op::PropertyGet(idx), span.clone());
            }
            Expr::New { class, args, span } => {
                self.compile_expr(class)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.emit(Op::New(args.len() as u8), span.clone());
            }
            Expr::Tell { object, method, args, span } => {
                self.compile_expr(object)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                let idx = self.add_string(method);
                self.emit(Op::Tell { name: idx, arg_count: args.len() as u8 }, span.clone());
            }
            Expr::Qualified { name, module, span } => {
                let module_idx = self.add_string(module);
                let name_idx = self.add_string(name);
                self.emit(Op::QualifiedGet(module_idx, name_idx), span.clone());
            }
            Expr::List(elems, _) => {
                for e in elems {
                    self.compile_expr(e)?;
                }
                self.emit(Op::BuildList(elems.len()), span);
            }
            Expr::Dict(pairs, _) => {
                for (k, v) in pairs {
                    self.compile_expr(k)?;
                    self.compile_expr(v)?;
                }
                self.emit(Op::BuildDict(pairs.len()), span);
            }
            _ => {
                return Err(CompileError("expression not supported by VM".to_string()));
            }
        }
        Ok(())
    }

    fn try_compile_range(&self, iterable: &Expr) -> Result<Option<(Option<Expr>, Option<Expr>, Option<Expr>)>, CompileError> {
        if let Expr::Call { callee, args, .. } = iterable {
            if let Expr::Variable { name, .. } = callee.as_ref() {
                if name == "range" && args.len() >= 1 && args.len() <= 3 {
                    return Ok(Some((args.get(0).cloned(), args.get(1).cloned(), args.get(2).cloned())));
                }
            }
        }
        Ok(None)
    }

    fn try_peephole_set(&self, slot: usize, value: &Expr) -> Option<Op> {
        if let Some(ref ann) = self.state().locals[slot].type_ann {
            if ann != "integer" && ann != "number" {
                return None;
            }
        }
        if let Expr::Binary { op: BinOp::Add, left, right, .. } = value {
            if Self::is_var_named(left, &self.state().locals[slot].name) && Self::is_literal_one(right) {
                return Some(Op::IncrementLocal(slot));
            }
            if Self::is_var_named(right, &self.state().locals[slot].name) && Self::is_literal_one(left) {
                return Some(Op::IncrementLocal(slot));
            }
            if Self::is_var_named(left, &self.state().locals[slot].name) {
                if let Some(y) = Self::var_name(right) {
                    if let Some(y_slot) = self.state().resolve_local(y) {
                        return Some(Op::AddLocals(slot, y_slot));
                    }
                }
            }
            if Self::is_var_named(right, &self.state().locals[slot].name) {
                if let Some(y) = Self::var_name(left) {
                    if let Some(y_slot) = self.state().resolve_local(y) {
                        return Some(Op::AddLocals(slot, y_slot));
                    }
                }
            }
        }
        None
    }

    fn declare_local_at_depth(&self, name: &str, type_ann: Option<String>, depth: usize) -> usize {
        self.state_mut().declare_local_at_depth(name, type_ann, depth)
    }

    fn is_var_named(expr: &Expr, name: &str) -> bool {
        matches!(expr, Expr::Variable { name: n, .. } if n == name)
    }

    fn var_name(expr: &Expr) -> Option<&str> {
        if let Expr::Variable { name, .. } = expr {
            Some(name)
        } else {
            None
        }
    }

    fn is_literal_one(expr: &Expr) -> bool {
        matches!(expr, Expr::Integer(n, _) if *n == num_bigint::BigInt::from(1))
            || matches!(expr, Expr::Number(n, _) if *n == 1.0)
    }
}

fn collect_captured_globals(stmts: &[Stmt]) -> HashSet<String> {
    let mut top_level = HashSet::new();
    for stmt in stmts {
        if let Stmt::Let { name, .. } = stmt {
            top_level.insert(name.clone());
        }
    }
    let mut captured = HashSet::new();
    for stmt in stmts {
        if let Stmt::Define { params, body, .. } = stmt {
            let mut locals: HashSet<String> = params.iter().map(|(n, _)| n.clone()).collect();
            scan_stmts_for_captured(body, &top_level, &mut locals, &mut captured);
        }
    }
    captured
}

fn scan_stmts_for_captured(stmts: &[Stmt], top_level: &HashSet<String>, locals: &mut HashSet<String>, captured: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { name, value, .. } => {
                scan_expr_for_captured(value, top_level, locals, captured);
                locals.insert(name.clone());
            }
            Stmt::Set { target, value } => {
                scan_expr_for_captured(value, top_level, locals, captured);
                if let AssignTarget::Index { object, index, .. } = target {
                    scan_expr_for_captured(object, top_level, locals, captured);
                    scan_expr_for_captured(index, top_level, locals, captured);
                }
                if let AssignTarget::Property { object, .. } = target {
                    scan_expr_for_captured(object, top_level, locals, captured);
                }
            }
            Stmt::Show(expr) | Stmt::Expr(expr) | Stmt::Return { value: Some(expr), .. } => {
                scan_expr_for_captured(expr, top_level, locals, captured);
            }
            Stmt::If { cond, then_branch, else_branch } => {
                scan_expr_for_captured(cond, top_level, locals, captured);
                scan_stmts_for_captured(then_branch, top_level, &mut locals.clone(), captured);
                scan_stmts_for_captured(else_branch, top_level, &mut locals.clone(), captured);
            }
            Stmt::While { cond, body } => {
                scan_expr_for_captured(cond, top_level, locals, captured);
                scan_stmts_for_captured(body, top_level, &mut locals.clone(), captured);
            }
            Stmt::For { var, iterable, body } => {
                scan_expr_for_captured(iterable, top_level, locals, captured);
                let mut body_locals = locals.clone();
                body_locals.insert(var.clone());
                scan_stmts_for_captured(body, top_level, &mut body_locals, captured);
            }
            Stmt::Try { body, catch_body, .. } => {
                scan_stmts_for_captured(body, top_level, &mut locals.clone(), captured);
                scan_stmts_for_captured(catch_body, top_level, &mut locals.clone(), captured);
            }
            Stmt::Return { value: None, .. } => {}
            _ => {}
        }
    }
}

fn scan_expr_for_captured(expr: &Expr, top_level: &HashSet<String>, locals: &HashSet<String>, captured: &mut HashSet<String>) {
    match expr {
        Expr::Variable { name, .. } => {
            if top_level.contains(name) && !locals.contains(name) {
                captured.insert(name.clone());
            }
        }
        Expr::Binary { left, right, .. } => {
            scan_expr_for_captured(left, top_level, locals, captured);
            scan_expr_for_captured(right, top_level, locals, captured);
        }
        Expr::Unary { operand, .. } => {
            scan_expr_for_captured(operand, top_level, locals, captured);
        }
        Expr::Call { callee, args, .. } => {
            scan_expr_for_captured(callee, top_level, locals, captured);
            for arg in args {
                scan_expr_for_captured(arg, top_level, locals, captured);
            }
        }
        Expr::Index { object, index, .. } => {
            scan_expr_for_captured(object, top_level, locals, captured);
            scan_expr_for_captured(index, top_level, locals, captured);
        }
        Expr::Property { object, .. } => {
            scan_expr_for_captured(object, top_level, locals, captured);
        }
        Expr::Tell { object, args, .. } => {
            scan_expr_for_captured(object, top_level, locals, captured);
            for arg in args {
                scan_expr_for_captured(arg, top_level, locals, captured);
            }
        }
        Expr::New { class, args, .. } => {
            scan_expr_for_captured(class, top_level, locals, captured);
            for arg in args {
                scan_expr_for_captured(arg, top_level, locals, captured);
            }
        }
        Expr::List(elems, _) => {
            for e in elems {
                scan_expr_for_captured(e, top_level, locals, captured);
            }
        }
        Expr::Dict(pairs, _) => {
            for (k, v) in pairs {
                scan_expr_for_captured(k, top_level, locals, captured);
                scan_expr_for_captured(v, top_level, locals, captured);
            }
        }
        _ => {}
    }
}

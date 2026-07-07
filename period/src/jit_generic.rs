//! Generic boxed-value JIT compiler.
//!
//! While the integer fast path in `jit.rs` keeps values as raw `i64`, this
//! compiler represents every Period value as an owned `*mut Value`.  Control
//! flow is native; arithmetic, comparisons, truthiness, etc. are delegated to
//! the C-ABI helpers in `jit_runtime.rs`.

use std::collections::{HashMap, HashSet};

use cranelift::prelude::*;
use cranelift::codegen::ir::{BlockArg, MemFlagsData, StackSlot};
use cranelift_module::{DataDescription, Linkage, Module};

use crate::ast::BinOp;
use crate::bytecode::{CompiledFunction, Op};
use crate::interpreter::Interpreter;
use crate::value::{Integer, Value};

type ClValue = cranelift::prelude::Value;

pub type GenericJitFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    upvalues: *mut std::ffi::c_void,
    argc: usize,
    argv: *const *mut Value,
) -> *mut Value;

#[repr(C)]
pub struct JitContext {
    pub interp: *mut Interpreter,
    pub function: *const CompiledFunction,
}

macro_rules! helper_ptr {
    ($f:expr) => {
        $f as *const () as *const u8
    };
}

pub struct GenericJitCompiler {
    pub(crate) module: cranelift_jit::JITModule,
    builder_context: FunctionBuilderContext,
    helpers: Helpers,
}

/// Information about a field-only class whose fields are all initialised directly
/// from constructor arguments.
#[derive(Clone)]
struct ClassInfo {
    field_names: Vec<usize>,
    /// Maps each field to the constructor argument index that initialises it.
    field_init: Vec<usize>,
}

/// A local slot that holds a single-field instance created by `new` and is only
/// ever used to read that field.  The instance allocation can be removed and the
/// field value used directly.
struct TransparentLocal {
    arg_idx: usize,
    field_name_idx: usize,
}

type ClassStackEntry = (Option<ClassInfo>, Option<usize>);

/// Analyse a function to find field-only classes and local slots that can be
/// replaced by a single constructor argument, avoiding instance allocation.
fn analyze_transparent_locals(
    func: &CompiledFunction,
) -> (
    HashMap<usize, ClassInfo>,
    HashMap<usize, TransparentLocal>,
    std::collections::HashSet<usize>,
    std::collections::HashSet<usize>,
) {
    let mut local_class: Vec<Option<ClassInfo>> = vec![None; func.local_count];
    let mut global_class: HashMap<usize, ClassInfo> = HashMap::new();
    let mut stack: Vec<ClassStackEntry> = Vec::new();
    let mut new_class_info: HashMap<usize, ClassInfo> = HashMap::new();
    let mut new_class_load_ip: HashMap<usize, usize> = HashMap::new();

    for (ip, op) in func.chunk.ops.iter().enumerate() {
        match op {
            Op::BuildClass {
                methods,
                init,
                fields,
                field_init,
                ..
            } => {
                let pops = methods.len() + if init.is_some() { 1 } else { 0 };
                for _ in 0..pops {
                    stack.pop();
                }
                let simple = field_init.iter().all(|&i| i != usize::MAX);
                if simple {
                    stack.push((
                        Some(ClassInfo {
                            field_names: fields.clone(),
                            field_init: field_init.clone(),
                        }),
                        Some(ip),
                    ));
                } else {
                    stack.push((None, None));
                }
            }
            Op::StoreLocal(slot) => {
                let (c, _) = stack.pop().unwrap_or((None, None));
                local_class[*slot] = c;
            }
            Op::StoreGlobal(_) | Op::SetUpvalue(_) => {
                stack.pop();
            }
            Op::DefineGlobal { name, .. } => {
                let (c, _) = stack.pop().unwrap_or((None, None));
                if let Some(c) = c {
                    global_class.insert(*name, c);
                }
            }
            Op::LoadLocal(slot) => {
                stack.push((local_class[*slot].clone(), Some(ip)));
            }
            Op::LoadGlobal(idx) => {
                stack.push((global_class.get(idx).cloned(), Some(ip)));
            }
            Op::Closure { .. }
            | Op::GetUpvalue(_)
            | Op::Constant(_)
            | Op::Nothing
            | Op::True
            | Op::False
            | Op::Import(_)
            | Op::QualifiedGet(_, _) => {
                stack.push((None, None));
            }
            Op::New(argc) => {
                let argc = *argc as usize;
                let class_pos = stack.len().saturating_sub(argc + 1);
                if let Some((Some(class), producer)) = stack.get(class_pos) {
                    new_class_info.insert(ip, class.clone());
                    if let Some(producer) = producer {
                        new_class_load_ip.insert(ip, *producer);
                    }
                }
                for _ in 0..=argc {
                    stack.pop();
                }
                stack.push((None, None));
            }
            Op::Dup => {
                stack.push(stack.last().cloned().unwrap_or((None, None)));
            }
            Op::Pop => {
                stack.pop();
            }
            _ => {
                let (pops, pushes) = op_stack_delta(op);
                for _ in 0..pops {
                    stack.pop();
                }
                for _ in 0..pushes {
                    stack.push((None, None));
                }
            }
        }
    }

    // Find locals whose only store comes from a `new` of a simple field-only
    // class and whose only loads are immediately followed by a property get of
    // the corresponding field.
    let mut stores: Vec<Vec<usize>> = vec![Vec::new(); func.local_count];
    let mut loads: Vec<Vec<usize>> = vec![Vec::new(); func.local_count];
    let mut other_refs: HashSet<usize> = HashSet::new();
    for (ip, op) in func.chunk.ops.iter().enumerate() {
        match op {
            Op::LoadLocal(slot) => loads[*slot].push(ip),
            Op::StoreLocal(slot) => stores[*slot].push(ip),
            Op::IncrementLocal(slot) => {
                other_refs.insert(*slot);
            }
            Op::AddLocals(target, source) => {
                other_refs.insert(*target);
                other_refs.insert(*source);
            }
            Op::Closure { upvalues, .. } => {
                for uv in upvalues {
                    if uv.is_local {
                        other_refs.insert(uv.index);
                    }
                }
            }
            _ => {}
        }
    }

    let mut transparent: HashMap<usize, TransparentLocal> = HashMap::new();
    let mut transparent_new_ips: std::collections::HashSet<usize> =
        std::collections::HashSet::new();
    let mut transparent_class_loads: std::collections::HashSet<usize> =
        std::collections::HashSet::new();

    for slot in 0..func.local_count {
        if other_refs.contains(&slot) {
            continue;
        }
        if stores[slot].len() != 1 {
            continue;
        }
        if loads[slot].is_empty() {
            continue;
        }
        let store_ip = stores[slot][0];
        if store_ip == 0 {
            continue;
        }
        let new_ip = store_ip - 1;
        let class = match new_class_info.get(&new_ip) {
            Some(c) => c,
            None => continue,
        };
        let mut arg_idx: Option<usize> = None;
        let mut field_name_idx: Option<usize> = None;
        let mut ok = true;
        for &load_ip in &loads[slot] {
            let get_ip = load_ip + 1;
            if get_ip >= func.chunk.ops.len() {
                ok = false;
                break;
            }
            let name_idx = match &func.chunk.ops[get_ip] {
                Op::PropertyGet(idx) => *idx,
                _ => {
                    ok = false;
                    break;
                }
            };
            let mut found = false;
            for (fi, &fname) in class.field_names.iter().enumerate() {
                if fname == name_idx {
                    let a = class.field_init[fi];
                    if a == usize::MAX {
                        ok = false;
                        break;
                    }
                    if arg_idx.is_none() {
                        arg_idx = Some(a);
                        field_name_idx = Some(name_idx);
                    } else if arg_idx != Some(a) || field_name_idx != Some(name_idx) {
                        ok = false;
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                ok = false;
            }
            if !ok {
                break;
            }
        }
        if !ok {
            continue;
        }
        if let (Some(a), Some(name)) = (arg_idx, field_name_idx) {
            if let Op::New(argc) = &func.chunk.ops[new_ip] {
                if a < *argc as usize {
                    transparent.insert(
                        slot,
                        TransparentLocal {
                            arg_idx: a,
                            field_name_idx: name,
                        },
                    );
                    transparent_new_ips.insert(new_ip);
                    if let Some(load_ip) = new_class_load_ip.get(&new_ip) {
                        transparent_class_loads.insert(*load_ip);
                    }
                }
            }
        }
    }

    (
        global_class,
        transparent,
        transparent_new_ips,
        transparent_class_loads,
    )
}

/// Abstract stack effect used for class dataflow tracking.
fn op_stack_delta(op: &Op) -> (usize, usize) {
    match op {
        Op::Constant(_)
        | Op::Nothing
        | Op::True
        | Op::False => (0, 1),
        Op::Pop => (1, 0),
        Op::Dup => (1, 2),
        Op::IncrementLocal(_)
        | Op::AddLocals(_, _)
        | Op::AppendLocalString { .. }
        | Op::CheckType(_)
        | Op::TryBegin(_, _)
        | Op::TryEnd
        | Op::Jump(_)
        | Op::Loop(_) => (0, 0),
        Op::AppendLocalList { .. } => (1, 0),
        Op::Binary(_)
        | Op::Index
        | Op::IndexSet => (2, 1),
        Op::Unary(_)
        | Op::JumpIfFalse(_)
        | Op::JumpIfTrue(_)
        | Op::IterInit
        | Op::Length
        | Op::Read => (1, 1),
        Op::Show | Op::Return => (1, 0),
        Op::Call(argc) | Op::New(argc) => (*argc as usize + 1, 1),
        Op::Tell { arg_count, .. } => (*arg_count as usize + 1, 1),
        Op::BuildList(n) => (*n as usize, 1),
        Op::BuildDict(n) => (*n as usize * 2, 1),
        Op::PropertyGet(_) => (1, 1),
        Op::PropertySet(_) => (2, 1),
        Op::Export(indices) => (indices.len(), 0),
        Op::Write => (2, 1),
        _ => (0, 0),
    }
}

/// Find catch-slot locals that are never read inside their catch bodies (or
/// anywhere else).  Their catch blocks do not need a value parameter.
fn analyze_unused_catch_slots(func: &CompiledFunction) -> HashSet<usize> {
    let mut catch_slots: HashSet<usize> = HashSet::new();
    let mut unused: HashSet<usize> = HashSet::new();
    for op in &func.chunk.ops {
        if let Op::TryBegin(_, slot) = op {
            catch_slots.insert(*slot);
        }
    }
    for slot in &catch_slots {
        let mut used = false;
        for op in &func.chunk.ops {
            match op {
                Op::LoadLocal(s) | Op::IncrementLocal(s) | Op::StoreLocal(s) => {
                    if s == slot {
                        used = true;
                        break;
                    }
                }
                Op::AddLocals(a, b) => {
                    if a == slot || b == slot {
                        used = true;
                        break;
                    }
                }
                Op::Closure { upvalues, .. } => {
                    if upvalues.iter().any(|u| u.is_local && u.index == *slot) {
                        used = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !used {
            unused.insert(*slot);
        }
    }
    unused
}

/// If this `Loop` is an append-only loop (`while i < n: s += chunk; i += 1`
/// or `while i < n: xs += [i]; i += 1`), emit a single bulk runtime call
/// that does the remaining iterations and jump to the loop exit.
fn try_optimize_append_loop(
    func: &CompiledFunction,
    i: usize,
    target: usize,
    builder: &mut FunctionBuilder,
    module: &mut cranelift_jit::JITModule,
    helpers: &Helpers,
    locals: &[Variable],
    interp_ptr: ClValue,
    blocks: &[Block],
    name_strings: &mut HashMap<usize, cranelift_module::DataId>,
) -> Option<()> {
    let _ = interp_ptr;
    let ops = &func.chunk.ops;
    if target + 4 >= ops.len() {
        return None;
    }

    // Loop header: LoadLocal(i) (Constant(n) | LoadLocal(n)) Lt JumpIfFalse(end)
    let i_slot = match &ops[target] {
        Op::LoadLocal(slot) => *slot,
        _ => return None,
    };
    let (n_raw, n_slot): (Option<ClValue>, Option<usize>) = match &ops[target + 1] {
        Op::Constant(idx) => {
            let c = &func.chunk.constants[*idx];
            let n = match c {
                Value::Integer(Integer::Small(n)) => *n,
                _ => return None,
            };
            (Some(builder.ins().iconst(types::I64, n)), None)
        }
        Op::LoadLocal(slot) => (None, Some(*slot)),
        _ => return None,
    };
    if !matches!(ops[target + 2], Op::Binary(BinOp::Lt)) {
        return None;
    }
    let end_ip = match &ops[target + 3] {
        Op::JumpIfFalse(ip) => *ip,
        _ => return None,
    };

    // Verify the loop body is side-effect free except for the append/increment.
    // We also ensure the bound local is not mutated inside the loop.
    let body_start = target + 4;
    for ip in body_start..i {
        match &ops[ip] {
            Op::StoreLocal(slot) | Op::IncrementLocal(slot) => {
                if let Some(ns) = n_slot {
                    if *slot == ns {
                        return None;
                    }
                }
            }
            Op::AppendLocalString { .. } | Op::AppendLocalList { .. } | Op::LoadLocal(_) => {}
            _ => return None,
        }
    }

    let i_after = builder.use_var(locals[i_slot]);
    let i_raw = call1(module, helpers, builder, helpers.as_i64, &[i_after]);
    let n_raw = match n_raw {
        Some(v) => v,
        None => {
            let ns = n_slot?;
            let n_local = builder.use_var(locals[ns]);
            call1(module, helpers, builder, helpers.as_i64, &[n_local])
        }
    };

    // count = n - i_current (remaining iterations after the current one)
    let count_raw = builder.ins().isub(n_raw, i_raw);

    let body_len = i - body_start;
    if body_len == 2
        && matches!(&ops[body_start + 1], Op::IncrementLocal(slot) if *slot == i_slot)
    {
        // while i < n: s += chunk; i += 1
        if let Op::AppendLocalString { slot, string_idx } = &ops[body_start] {
            let local_ptr = builder.use_var(locals[*slot]);
            let (ptr, len_val) = emit_string(module, helpers, builder, name_strings, &func.chunk.strings[*string_idx], *string_idx);
            call(module, helpers, builder, helpers.append_string_repeat, &[local_ptr, ptr, len_val, count_raw]);
        } else {
            return None;
        }
    } else if body_len == 3
        && matches!(&ops[body_start + 2], Op::IncrementLocal(slot) if *slot == i_slot)
    {
        // while i < n: xs += [i]; i += 1
        if let (Op::LoadLocal(load_slot), Op::AppendLocalList { slot }) = (&ops[body_start], &ops[body_start + 1]) {
            if *load_slot != i_slot {
                return None;
            }
            let local_ptr = builder.use_var(locals[*slot]);
            call(module, helpers, builder, helpers.append_list_range, &[local_ptr, i_raw, count_raw]);
        } else {
            return None;
        }
    } else {
        return None;
    }

    // i = n (as a boxed value)
    let n_val = call1(module, helpers, builder, helpers.from_i64, &[n_raw]);
    let old_i = builder.use_var(locals[i_slot]);
    builder.def_var(locals[i_slot], n_val);
    drop_value(module, helpers, builder, old_i);

    let end_block = *blocks.get(end_ip)?;
    builder.ins().jump(end_block, &[]);
    Some(())
}

struct CachedJitCode {
    code: GenericJitFn,
    _module: Box<cranelift_jit::JITModule>,
}

thread_local! {
    static JIT_CODE_CACHE: std::cell::RefCell<HashMap<*const CompiledFunction, CachedJitCode>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Return the cached JIT code pointer for a compiled function, compiling it on
/// first use. The backing JIT module is leaked so the generated code stays valid
/// for the lifetime of the process.
pub fn get_jit_code(func: &CompiledFunction) -> Option<GenericJitFn> {
    let key = func as *const CompiledFunction;
    JIT_CODE_CACHE.with(|cache| {
        {
            let borrow = cache.borrow();
            if let Some(cached) = borrow.get(&key) {
                return Some(cached.code);
            }
        }
        let mut compiler = GenericJitCompiler::new();
        let code = compiler.compile(func)?;
        let cached = CachedJitCode {
            code,
            _module: Box::new(compiler.module),
        };
        cache.borrow_mut().insert(key, cached);
        Some(code)
    })
}

struct Helpers {
    clone: cranelift_module::FuncId,
    drop: cranelift_module::FuncId,
    is_error: cranelift_module::FuncId,
    set_span: cranelift_module::FuncId,
    from_i64: cranelift_module::FuncId,
    as_i64: cranelift_module::FuncId,
    from_f64: cranelift_module::FuncId,
    from_bool: cranelift_module::FuncId,
    nothing: cranelift_module::FuncId,
    from_string: cranelift_module::FuncId,
    truthy: cranelift_module::FuncId,
    show: cranelift_module::FuncId,
    binary: cranelift_module::FuncId,
    unary: cranelift_module::FuncId,
    env_get: cranelift_module::FuncId,
    env_set: cranelift_module::FuncId,
    env_define: cranelift_module::FuncId,
    raise: cranelift_module::FuncId,
    build_list: cranelift_module::FuncId,
    build_dict: cranelift_module::FuncId,
    index_get: cranelift_module::FuncId,
    index_set: cranelift_module::FuncId,
    property_get: cranelift_module::FuncId,
    property_set: cranelift_module::FuncId,
    length: cranelift_module::FuncId,
    iter_init: cranelift_module::FuncId,
    call: cranelift_module::FuncId,
    tell: cranelift_module::FuncId,
    new_instance: cranelift_module::FuncId,
    build_class: cranelift_module::FuncId,
    make_closure: cranelift_module::FuncId,
    upvalue_alloc: cranelift_module::FuncId,
    upvalue_get: cranelift_module::FuncId,
    upvalue_set: cranelift_module::FuncId,
    import: cranelift_module::FuncId,
    qualified_get: cranelift_module::FuncId,
    export_name: cranelift_module::FuncId,
    read: cranelift_module::FuncId,
    write: cranelift_module::FuncId,
    check_type: cranelift_module::FuncId,
    append_local_string: cranelift_module::FuncId,
    append_local_list: cranelift_module::FuncId,
    append_string_repeat: cranelift_module::FuncId,
    append_list_range: cranelift_module::FuncId,
    increment_local: cranelift_module::FuncId,
    add_locals: cranelift_module::FuncId,
}

impl GenericJitCompiler {
    pub fn new() -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        flag_builder.set("opt_level", "speed").unwrap();
        let isa_builder = cranelift_native::builder().unwrap_or_else(|msg| {
            panic!("host machine is not supported: {}", msg);
        });
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let mut builder =
            cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

        builder.symbol("period_value_clone", helper_ptr!(crate::jit_runtime::period_value_clone));
        builder.symbol("period_value_drop", helper_ptr!(crate::jit_runtime::period_value_drop));
        builder.symbol("period_value_is_error", helper_ptr!(crate::jit_runtime::period_value_is_error));
        builder.symbol("period_set_span", helper_ptr!(crate::jit_runtime::period_set_span));
        builder.symbol("period_value_from_i64", helper_ptr!(crate::jit_runtime::period_value_from_i64));
        builder.symbol("period_value_as_i64", helper_ptr!(crate::jit_runtime::period_value_as_i64));
        builder.symbol("period_value_from_f64", helper_ptr!(crate::jit_runtime::period_value_from_f64));
        builder.symbol("period_value_from_bool", helper_ptr!(crate::jit_runtime::period_value_from_bool));
        builder.symbol("period_value_nothing", helper_ptr!(crate::jit_runtime::period_value_nothing));
        builder.symbol("period_value_from_string", helper_ptr!(crate::jit_runtime::period_value_from_string));
        builder.symbol("period_value_truthy", helper_ptr!(crate::jit_runtime::period_value_truthy));
        builder.symbol("period_value_show", helper_ptr!(crate::jit_runtime::period_value_show));
        builder.symbol("period_value_binary", helper_ptr!(crate::jit_runtime::period_value_binary));
        builder.symbol("period_value_unary", helper_ptr!(crate::jit_runtime::period_value_unary));
        builder.symbol("period_env_get", helper_ptr!(crate::jit_runtime::period_env_get));
        builder.symbol("period_env_set", helper_ptr!(crate::jit_runtime::period_env_set));
        builder.symbol("period_env_define", helper_ptr!(crate::jit_runtime::period_env_define));
        builder.symbol("period_raise", helper_ptr!(crate::jit_runtime::period_raise));
        builder.symbol("period_build_list", helper_ptr!(crate::jit_runtime::period_build_list));
        builder.symbol("period_build_dict", helper_ptr!(crate::jit_runtime::period_build_dict));
        builder.symbol("period_index_get", helper_ptr!(crate::jit_runtime::period_index_get));
        builder.symbol("period_index_set", helper_ptr!(crate::jit_runtime::period_index_set));
        builder.symbol("period_property_get", helper_ptr!(crate::jit_runtime::period_property_get));
        builder.symbol("period_property_set", helper_ptr!(crate::jit_runtime::period_property_set));
        builder.symbol("period_length", helper_ptr!(crate::jit_runtime::period_length));
        builder.symbol("period_iter_init", helper_ptr!(crate::jit_runtime::period_iter_init));
        builder.symbol("period_call", helper_ptr!(crate::jit_runtime::period_call));
        builder.symbol("period_tell", helper_ptr!(crate::jit_runtime::period_tell));
        builder.symbol("period_new_instance", helper_ptr!(crate::jit_runtime::period_new_instance));
        builder.symbol("period_build_class", helper_ptr!(crate::jit_runtime::period_build_class));
        builder.symbol("period_make_closure", helper_ptr!(crate::jit_runtime::period_make_closure));
        builder.symbol("period_upvalue_alloc", helper_ptr!(crate::jit_runtime::period_upvalue_alloc));
        builder.symbol("period_upvalue_get", helper_ptr!(crate::jit_runtime::period_upvalue_get));
        builder.symbol("period_upvalue_set", helper_ptr!(crate::jit_runtime::period_upvalue_set));
        builder.symbol("period_import", helper_ptr!(crate::jit_runtime::period_import));
        builder.symbol("period_qualified_get", helper_ptr!(crate::jit_runtime::period_qualified_get));
        builder.symbol("period_export_name", helper_ptr!(crate::jit_runtime::period_export_name));
        builder.symbol("period_read", helper_ptr!(crate::jit_runtime::period_read));
        builder.symbol("period_write", helper_ptr!(crate::jit_runtime::period_write));
        builder.symbol("period_check_type", helper_ptr!(crate::jit_runtime::period_check_type));
        builder.symbol("period_append_local_string", helper_ptr!(crate::jit_runtime::period_append_local_string));
        builder.symbol("period_append_local_list", helper_ptr!(crate::jit_runtime::period_append_local_list));
        builder.symbol("period_string_append_repeat", helper_ptr!(crate::jit_runtime::period_string_append_repeat));
        builder.symbol("period_list_append_range", helper_ptr!(crate::jit_runtime::period_list_append_range));
        builder.symbol("period_value_increment_local", helper_ptr!(crate::jit_runtime::period_value_increment_local));
        builder.symbol("period_value_add_locals", helper_ptr!(crate::jit_runtime::period_value_add_locals));

        let mut module = cranelift_jit::JITModule::new(builder);

        let helpers = Helpers {
            clone: declare_helper(&mut module, "period_value_clone", &[types::I64], &[types::I64]),
            drop: declare_helper(&mut module, "period_value_drop", &[types::I64], &[]),
            is_error: declare_helper(&mut module, "period_value_is_error", &[types::I64], &[types::I64]),
            set_span: declare_helper(&mut module, "period_set_span", &[types::I64, types::I64], &[]),
            from_i64: declare_helper(&mut module, "period_value_from_i64", &[types::I64], &[types::I64]),
            as_i64: declare_helper(&mut module, "period_value_as_i64", &[types::I64], &[types::I64]),
            from_f64: declare_helper(&mut module, "period_value_from_f64", &[types::F64], &[types::I64]),
            from_bool: declare_helper(&mut module, "period_value_from_bool", &[types::I64], &[types::I64]),
            nothing: declare_helper(&mut module, "period_value_nothing", &[], &[types::I64]),
            from_string: declare_helper(&mut module, "period_value_from_string", &[types::I64, types::I64], &[types::I64]),
            truthy: declare_helper(&mut module, "period_value_truthy", &[types::I64], &[types::I64]),
            show: declare_helper(&mut module, "period_value_show", &[types::I64, types::I64], &[]),
            binary: declare_helper(&mut module, "period_value_binary", &[types::I8, types::I64, types::I64], &[types::I64]),
            unary: declare_helper(&mut module, "period_value_unary", &[types::I8, types::I64], &[types::I64]),
            env_get: declare_helper(&mut module, "period_env_get", &[types::I64, types::I64, types::I64], &[types::I64]),
            env_set: declare_helper(&mut module, "period_env_set", &[types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            env_define: declare_helper(&mut module, "period_env_define", &[types::I64, types::I64, types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            raise: declare_helper(&mut module, "period_raise", &[types::I64], &[types::I64]),
            build_list: declare_helper(&mut module, "period_build_list", &[types::I64, types::I64], &[types::I64]),
            build_dict: declare_helper(&mut module, "period_build_dict", &[types::I64, types::I64], &[types::I64]),
            index_get: declare_helper(&mut module, "period_index_get", &[types::I64, types::I64], &[types::I64]),
            index_set: declare_helper(&mut module, "period_index_set", &[types::I64, types::I64, types::I64], &[types::I64]),
            property_get: declare_helper(&mut module, "period_property_get", &[types::I64, types::I64, types::I64], &[types::I64]),
            property_set: declare_helper(&mut module, "period_property_set", &[types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            length: declare_helper(&mut module, "period_length", &[types::I64], &[types::I64]),
            iter_init: declare_helper(&mut module, "period_iter_init", &[types::I64], &[types::I64]),
            call: declare_helper(&mut module, "period_call", &[types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            tell: declare_helper(&mut module, "period_tell", &[types::I64, types::I64, types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            new_instance: declare_helper(&mut module, "period_new_instance", &[types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            build_class: declare_helper(&mut module, "period_build_class", &[types::I64, types::I64, types::I64, types::I64, types::I64, types::I64, types::I64, types::I64, types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            make_closure: declare_helper(&mut module, "period_make_closure", &[types::I64, types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            upvalue_alloc: declare_helper(&mut module, "period_upvalue_alloc", &[types::I64], &[types::I64]),
            upvalue_get: declare_helper(&mut module, "period_upvalue_get", &[types::I64], &[types::I64]),
            upvalue_set: declare_helper(&mut module, "period_upvalue_set", &[types::I64, types::I64], &[]),
            import: declare_helper(&mut module, "period_import", &[types::I64, types::I64, types::I64], &[types::I64]),
            qualified_get: declare_helper(&mut module, "period_qualified_get", &[types::I64, types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            export_name: declare_helper(&mut module, "period_export_name", &[types::I64, types::I64, types::I64], &[types::I64]),
            read: declare_helper(&mut module, "period_read", &[types::I64, types::I64], &[types::I64]),
            write: declare_helper(&mut module, "period_write", &[types::I64, types::I64, types::I64], &[types::I64]),
            check_type: declare_helper(&mut module, "period_check_type", &[types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            append_local_string: declare_helper(&mut module, "period_append_local_string", &[types::I64, types::I64, types::I64], &[types::I64]),
            append_local_list: declare_helper(&mut module, "period_append_local_list", &[types::I64, types::I64], &[types::I64]),
            append_string_repeat: declare_helper(&mut module, "period_string_append_repeat", &[types::I64, types::I64, types::I64, types::I64], &[types::I64]),
            append_list_range: declare_helper(&mut module, "period_list_append_range", &[types::I64, types::I64, types::I64], &[types::I64]),
            increment_local: declare_helper(&mut module, "period_value_increment_local", &[types::I64], &[types::I64]),
            add_locals: declare_helper(&mut module, "period_value_add_locals", &[types::I64, types::I64], &[types::I64]),
        };

        Self {
            module,
            builder_context: FunctionBuilderContext::new(),
            helpers,
        }
    }

    pub fn compile(&mut self, func: &CompiledFunction) -> Option<GenericJitFn> {
        let dump = std::env::var("PERIOD_JIT_DUMP").is_ok();
        if dump {
            eprintln!("GENERIC JIT compiling '{}' locals={}", func.name, func.local_count);
            for (i, op) in func.chunk.ops.iter().enumerate() {
                let sp = &func.chunk.spans[i];
                eprintln!("{:3}: {:?}  span={}:{}", i, op, sp.line, sp.col);
            }
        }

        if !self.is_supported(func) {
            return None;
        }

        let (_global_class, transparent_locals, _transparent_new_ips, transparent_class_loads) =
            analyze_transparent_locals(func);
        let unused_catch_slots = analyze_unused_catch_slots(func);

        let mut ctx = self.module.make_context();
        ctx.func.signature.params.push(AbiParam::new(types::I64));
        ctx.func.signature.params.push(AbiParam::new(types::I64));
        ctx.func.signature.params.push(AbiParam::new(types::I64));
        ctx.func.signature.params.push(AbiParam::new(types::I64));
        ctx.func.signature.returns.push(AbiParam::new(types::I64));

        let ops = &func.chunk.ops;
        let n = ops.len();

        let bc = &mut self.builder_context;
        let module = &mut self.module;
        let helpers = &self.helpers;
        let mut builder = FunctionBuilder::new(&mut ctx.func, bc);
        let blocks: Vec<Block> = (0..n).map(|_| builder.create_block()).collect();
        let entry = blocks[0];
        builder.switch_to_block(entry);
        builder.append_block_params_for_function_params(entry);

        let ctx_param = builder.block_params(entry)[0];
        let upvalues_param = builder.block_params(entry)[1];
        let _argc_param = builder.block_params(entry)[2];
        let argv_param = builder.block_params(entry)[3];
        let interp_ptr = builder.ins().load(types::I64, MemFlagsData::new(), ctx_param, 0);
        let function_ptr = builder.ins().load(types::I64, MemFlagsData::new(), ctx_param, 8);

        // Every local is an owned pointer.  Initialise them to Nothing.
        let mut locals: Vec<Variable> = Vec::with_capacity(func.local_count);
        for _ in 0..func.local_count {
            locals.push(builder.declare_var(types::I64));
        }
        let mut captured_locals: HashSet<usize> = HashSet::new();
        for op in ops {
            if let Op::Closure { upvalues, .. } = op {
                for uv in upvalues {
                    if uv.is_local {
                        captured_locals.insert(uv.index);
                    }
                }
            }
        }
        for (slot, var) in locals.iter().enumerate() {
            let nothing_val = call0(module, helpers, &mut builder, helpers.nothing, &[]);
            if captured_locals.contains(&slot) {
                let cell = call1(module, helpers, &mut builder, helpers.upvalue_alloc, &[nothing_val]);
                builder.def_var(*var, cell);
            } else {
                builder.def_var(*var, nothing_val);
            }
        }
        // Bind arguments to parameter locals.
        for (slot, _) in func.params.iter().enumerate() {
            let offset = (slot * 8) as i32;
            let arg = builder.ins().load(types::I64, MemFlagsData::new(), argv_param, offset);
            if captured_locals.contains(&slot) {
                let cell = builder.use_var(locals[slot]);
                call_void(module, helpers, &mut builder, helpers.upvalue_set, &[cell, arg]);
            } else {
                builder.def_var(locals[slot], arg);
            }
        }

        let mut const_strings: HashMap<usize, cranelift_module::DataId> = HashMap::new();
        let mut name_strings: HashMap<usize, cranelift_module::DataId> = HashMap::new();
        let mut stack: Vec<ClValue> = Vec::new();
        let error_block = builder.create_block();
        builder.append_block_param(error_block, types::I64);
        let mut try_stack: Vec<(Block, usize, usize)> = Vec::new();
        let mut pending_catches: Vec<(Block, usize, usize)> = Vec::new();
        let mut extra_blocks: Vec<Block> = Vec::new();

        for (i, op) in ops.iter().enumerate() {
            let block = blocks[i];
            if builder.current_block() != Some(block) {
                builder.switch_to_block(block);
            }
            let span = &func.chunk.spans[i];
            set_span(module, helpers, &mut builder, span.line as i64, span.col as i64);
            let handler = try_stack.last().map(|(b, _, _)| *b).unwrap_or(error_block);

            let mut terminated = false;
            match op {
                Op::Constant(idx) => {
                    let v = emit_constant(module, helpers, &mut builder, &mut const_strings, &func.chunk.constants[*idx], *idx)?;
                    stack.push(v);
                }
                Op::Nothing => {
                    stack.push(call0(module, helpers, &mut builder, helpers.nothing, &[]));
                }
                Op::True => {
                    let one = builder.ins().iconst(types::I64, 1);
                    stack.push(call1(module, helpers, &mut builder, helpers.from_bool, &[one]));
                }
                Op::False => {
                    let zero = builder.ins().iconst(types::I64, 0);
                    stack.push(call1(module, helpers, &mut builder, helpers.from_bool, &[zero]));
                }
                Op::Pop => {
                    let v = stack.pop()?;
                    drop_value(module, helpers, &mut builder, v);
                }
                Op::Dup => {
                    let v = *stack.last()?;
                    let cloned = call1(module, helpers, &mut builder, helpers.clone, &[v]);
                    stack.push(cloned);
                }
                Op::LoadLocal(slot) => {
                    if transparent_class_loads.contains(&i) {
                        // This local only serves as the class operand for a
                        // transparent `new`.  Push a placeholder; the real
                        // value is never inspected.
                        stack.push(builder.ins().iconst(types::I64, 0));
                    } else if captured_locals.contains(slot) {
                        let v = builder.use_var(locals[*slot]);
                        let value = call1(module, helpers, &mut builder, helpers.upvalue_get, &[v]);
                        stack.push(value);
                    } else {
                        let v = builder.use_var(locals[*slot]);
                        let cloned = call1(module, helpers, &mut builder, helpers.clone, &[v]);
                        stack.push(cloned);
                    }
                }
                Op::StoreLocal(slot) => {
                    let v = stack.pop()?;
                    if captured_locals.contains(slot) {
                        let cell = builder.use_var(locals[*slot]);
                        call_void(module, helpers, &mut builder, helpers.upvalue_set, &[cell, v]);
                    } else {
                        let old = builder.use_var(locals[*slot]);
                        builder.def_var(locals[*slot], v);
                        drop_value(module, helpers, &mut builder, old);
                    }
                }
                Op::IncrementLocal(slot) => {
                    if !captured_locals.contains(slot) {
                        let local_ptr = builder.use_var(locals[*slot]);
                        let ok = call1(module, helpers, &mut builder, helpers.increment_local, &[local_ptr]);
                        let cmp = builder.ins().icmp_imm(IntCC::Equal, ok, 1);
                        let fallthrough = blocks.get(i + 1).copied()?;
                        let fallback = builder.create_block();
                        builder.ins().brif(cmp, fallthrough, &[], fallback, &[]);
                        builder.switch_to_block(fallback);
                        let cur = local_ptr;
                        let one_const = builder.ins().iconst(types::I64, 1);
                        let one = call1(module, helpers, &mut builder, helpers.from_i64, &[one_const]);
                        let op_val = builder.ins().iconst(types::I8, BinOp::Add as u8 as i64);
                        let sum = call(module, helpers, &mut builder, helpers.binary, &[op_val, cur, one]);
                        drop_value(module, helpers, &mut builder, one);
                        builder.def_var(locals[*slot], sum);
                        drop_value(module, helpers, &mut builder, cur);
                        builder.ins().jump(fallthrough, &[]);
                        extra_blocks.push(fallback);
                        terminated = true;
                    } else {
                        let cell_or_value = builder.use_var(locals[*slot]);
                        let cur = call1(module, helpers, &mut builder, helpers.upvalue_get, &[cell_or_value]);
                        let one_const = builder.ins().iconst(types::I64, 1);
                        let one = call1(module, helpers, &mut builder, helpers.from_i64, &[one_const]);
                        let op_val = builder.ins().iconst(types::I8, BinOp::Add as u8 as i64);
                        let sum = call(module, helpers, &mut builder, helpers.binary, &[op_val, cur, one]);
                        drop_value(module, helpers, &mut builder, one);
                        call_void(module, helpers, &mut builder, helpers.upvalue_set, &[cell_or_value, sum]);
                        drop_value(module, helpers, &mut builder, cur);
                    }
                }
                Op::AddLocals(target, source) => {
                    if !captured_locals.contains(target) && !captured_locals.contains(source) {
                        let target_ptr = builder.use_var(locals[*target]);
                        let src_ptr = builder.use_var(locals[*source]);
                        let ok = call1(module, helpers, &mut builder, helpers.add_locals, &[target_ptr, src_ptr]);
                        let cmp = builder.ins().icmp_imm(IntCC::Equal, ok, 1);
                        let fallthrough = blocks.get(i + 1).copied()?;
                        let fallback = builder.create_block();
                        builder.ins().brif(cmp, fallthrough, &[], fallback, &[]);
                        builder.switch_to_block(fallback);
                        let cur = target_ptr;
                        let cloned_src = call1(module, helpers, &mut builder, helpers.clone, &[src_ptr]);
                        let op_val = builder.ins().iconst(types::I8, BinOp::Add as u8 as i64);
                        let sum = call(module, helpers, &mut builder, helpers.binary, &[op_val, cur, cloned_src]);
                        drop_value(module, helpers, &mut builder, cloned_src);
                        builder.def_var(locals[*target], sum);
                        drop_value(module, helpers, &mut builder, cur);
                        builder.ins().jump(fallthrough, &[]);
                        extra_blocks.push(fallback);
                        terminated = true;
                    } else {
                        let target_cell_or_value = builder.use_var(locals[*target]);
                        let cur = if captured_locals.contains(target) {
                            call1(module, helpers, &mut builder, helpers.upvalue_get, &[target_cell_or_value])
                        } else {
                            target_cell_or_value
                        };
                        let src_cell_or_value = builder.use_var(locals[*source]);
                        let src = if captured_locals.contains(source) {
                            call1(module, helpers, &mut builder, helpers.upvalue_get, &[src_cell_or_value])
                        } else {
                            src_cell_or_value
                        };
                        let cloned_src = if captured_locals.contains(source) {
                            src // already a fresh clone from the cell
                        } else {
                            call1(module, helpers, &mut builder, helpers.clone, &[src])
                        };
                        let op_val = builder.ins().iconst(types::I8, BinOp::Add as u8 as i64);
                        let sum = call(module, helpers, &mut builder, helpers.binary, &[op_val, cur, cloned_src]);
                        drop_value(module, helpers, &mut builder, cloned_src);
                        if captured_locals.contains(target) {
                            call_void(module, helpers, &mut builder, helpers.upvalue_set, &[target_cell_or_value, sum]);
                            drop_value(module, helpers, &mut builder, cur);
                        } else {
                            builder.def_var(locals[*target], sum);
                            drop_value(module, helpers, &mut builder, cur);
                        }
                    }
                }
                Op::AppendLocalString { slot, string_idx } => {
                    if captured_locals.contains(slot) {
                        let cell = builder.use_var(locals[*slot]);
                        let cur = call1(module, helpers, &mut builder, helpers.upvalue_get, &[cell]);
                        let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*string_idx], *string_idx);
                        let right = call(module, helpers, &mut builder, helpers.from_string, &[ptr, len]);
                        let op_val = builder.ins().iconst(types::I8, BinOp::Add as u8 as i64);
                        let sum = call(module, helpers, &mut builder, helpers.binary, &[op_val, cur, right]);
                        guard_error(module, helpers, &mut builder, sum, handler);
                        drop_value(module, helpers, &mut builder, cur);
                        drop_value(module, helpers, &mut builder, right);
                        call_void(module, helpers, &mut builder, helpers.upvalue_set, &[cell, sum]);
                    } else {
                        let local_ptr = builder.use_var(locals[*slot]);
                        let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*string_idx], *string_idx);
                        // append_local_string returns null on success for these
                        // direct-local cases, so the error guard can be skipped.
                        let _res = call(module, helpers, &mut builder, helpers.append_local_string, &[local_ptr, ptr, len]);
                    }
                }
                Op::AppendLocalList { slot } => {
                    if captured_locals.contains(slot) {
                        let item = stack.pop()?;
                        let (slot_data, argv_ptr) = store_in_stack_slot(&mut builder, &[item]);
                        let _ = slot_data;
                        let count_val = builder.ins().iconst(types::I64, 1);
                        let list = call(module, helpers, &mut builder, helpers.build_list, &[count_val, argv_ptr]);
                        guard_error(module, helpers, &mut builder, list, handler);

                        let cell = builder.use_var(locals[*slot]);
                        let cur = call1(module, helpers, &mut builder, helpers.upvalue_get, &[cell]);
                        let op_val = builder.ins().iconst(types::I8, BinOp::Add as u8 as i64);
                        let sum = call(module, helpers, &mut builder, helpers.binary, &[op_val, cur, list]);
                        guard_error(module, helpers, &mut builder, sum, handler);
                        drop_value(module, helpers, &mut builder, cur);
                        drop_value(module, helpers, &mut builder, list);
                        call_void(module, helpers, &mut builder, helpers.upvalue_set, &[cell, sum]);
                    } else {
                        let item = stack.pop()?;
                        let local_ptr = builder.use_var(locals[*slot]);
                        // append_local_list returns null on success for these
                        // direct-local cases, so the error guard can be skipped.
                        let _res = call(module, helpers, &mut builder, helpers.append_local_list, &[local_ptr, item]);
                    }
                }
                Op::LoadGlobal(idx) => {
                    if transparent_class_loads.contains(&i) {
                        // This global only serves as the class operand for a
                        // transparent `new`.  Push a placeholder; the real
                        // value is never inspected.
                        stack.push(builder.ins().iconst(types::I64, 0));
                    } else {
                        let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*idx], *idx);
                        let v = call(module, helpers, &mut builder, helpers.env_get, &[interp_ptr, ptr, len]);
                        stack.push(v);
                        guard_error(module, helpers, &mut builder, v, handler);
                    }
                }
                Op::StoreGlobal(idx) => {
                    let v = stack.pop()?;
                    let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*idx], *idx);
                    let res = call(module, helpers, &mut builder, helpers.env_set, &[interp_ptr, ptr, len, v]);
                    guard_error(module, helpers, &mut builder, res, handler);
                    drop_value(module, helpers, &mut builder, res);
                }
                Op::DefineGlobal { name, type_ann } => {
                    let v = stack.pop()?;
                    let (name_ptr, name_len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*name], *name);
                    let (ann_ptr, ann_len) = if let Some(ann) = type_ann {
                        let (p, l) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*ann], *ann);
                        (p, l)
                    } else {
                        (builder.ins().iconst(types::I64, 0), builder.ins().iconst(types::I64, 0))
                    };
                    let res = call(module, helpers, &mut builder, helpers.env_define, &[interp_ptr, name_ptr, name_len, v, ann_ptr, ann_len]);
                    guard_error(module, helpers, &mut builder, res, handler);
                    drop_value(module, helpers, &mut builder, res);
                }
                Op::Closure { func: fidx, upvalues } => {
                    let uv_count = upvalues.len();
                    let mut uv_ptrs: Vec<ClValue> = Vec::with_capacity(uv_count);
                    for uv in upvalues {
                        let ptr = if uv.is_local {
                            builder.use_var(locals[uv.index])
                        } else {
                            let offset = (uv.index * 8) as i32;
                            builder.ins().load(types::I64, MemFlagsData::new(), upvalues_param, offset)
                        };
                        uv_ptrs.push(ptr);
                    }
                    let (slot, uv_ptr) = store_in_stack_slot(&mut builder, &uv_ptrs);
                    let _ = slot;
                    let idx = builder.ins().iconst(types::I64, *fidx as i64);
                    let uv_count_val = builder.ins().iconst(types::I64, uv_count as i64);
                    let closure = call(module, helpers, &mut builder, helpers.make_closure, &[interp_ptr, function_ptr, idx, uv_count_val, uv_ptr]);
                    stack.push(closure);
                    guard_error(module, helpers, &mut builder, closure, handler);
                }
                Op::GetUpvalue(idx) => {
                    let offset = (*idx * 8) as i32;
                    let cell = builder.ins().load(types::I64, MemFlagsData::new(), upvalues_param, offset);
                    let v = call1(module, helpers, &mut builder, helpers.upvalue_get, &[cell]);
                    stack.push(v);
                }
                Op::SetUpvalue(idx) => {
                    let v = stack.pop()?;
                    let offset = (*idx * 8) as i32;
                    let cell = builder.ins().load(types::I64, MemFlagsData::new(), upvalues_param, offset);
                    call_void(module, helpers, &mut builder, helpers.upvalue_set, &[cell, v]);
                }
                Op::Binary(bin_op) => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let op_val = builder.ins().iconst(types::I8, *bin_op as u8 as i64);
                    let res = call(module, helpers, &mut builder, helpers.binary, &[op_val, left, right]);
                    stack.push(res);
                    drop_value(module, helpers, &mut builder, left);
                    drop_value(module, helpers, &mut builder, right);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::Unary(unary_op) => {
                    let v = stack.pop()?;
                    let op_val = builder.ins().iconst(types::I8, *unary_op as u8 as i64);
                    let res = call(module, helpers, &mut builder, helpers.unary, &[op_val, v]);
                    stack.push(res);
                    drop_value(module, helpers, &mut builder, v);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::JumpIfFalse(target) => {
                    let cond = stack.pop()?;
                    let t = call1(module, helpers, &mut builder, helpers.truthy, &[cond]);
                    drop_value(module, helpers, &mut builder, cond);
                    let cmp = builder.ins().icmp_imm(IntCC::NotEqual, t, 0);
                    let fallthrough = blocks.get(i + 1).copied()?;
                    let dest = blocks.get(*target).copied()?;
                    builder.ins().brif(cmp, fallthrough, &[], dest, &[]);
                    terminated = true;
                }
                Op::JumpIfTrue(target) => {
                    let cond = stack.pop()?;
                    let t = call1(module, helpers, &mut builder, helpers.truthy, &[cond]);
                    drop_value(module, helpers, &mut builder, cond);
                    let cmp = builder.ins().icmp_imm(IntCC::NotEqual, t, 0);
                    let dest = blocks.get(*target).copied()?;
                    let fallthrough = blocks.get(i + 1).copied()?;
                    builder.ins().brif(cmp, dest, &[], fallthrough, &[]);
                    terminated = true;
                }
                Op::Jump(target) => {
                    let dest = blocks.get(*target).copied()?;
                    builder.ins().jump(dest, &[]);
                    terminated = true;
                }
                Op::Loop(target) => {
                    if try_optimize_append_loop(
                        func, i, *target, &mut builder, module, helpers, &locals, interp_ptr, &blocks, &mut name_strings,
                    )
                    .is_some()
                    {
                        terminated = true;
                    } else {
                        let dest = blocks.get(*target).copied()?;
                        builder.ins().jump(dest, &[]);
                        terminated = true;
                    }
                }
                Op::Call(arg_count) => {
                    let argc = *arg_count as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(stack.pop()?);
                    }
                    args.reverse();
                    let callee = stack.pop()?;
                    // Fast path for the global `error` builtin: build an error
                    // value directly and branch to the active handler instead of
                    // going through the full call dispatch machinery.
                    let is_error_call = if argc == 1 {
                        func.chunk
                            .ops
                            .get(i.saturating_sub(argc + 1))
                            .map_or(false, |op| match op {
                                Op::LoadGlobal(name) => {
                                    func.chunk.strings.get(*name).map_or(false, |s| s == "error")
                                }
                                _ => false,
                            })
                    } else {
                        false
                    };
                    if is_error_call {
                        drop_value(module, helpers, &mut builder, callee);
                        let msg = args.into_iter().next().unwrap();
                        let err = call1(module, helpers, &mut builder, helpers.raise, &[msg]);
                        stack.push(err);
                        guard_error(module, helpers, &mut builder, err, handler);
                    } else {
                        let (_slot, argv_ptr) = store_in_stack_slot(&mut builder, &args);
                        let argc_val = builder.ins().iconst(types::I64, argc as i64);
                        let res = call(module, helpers, &mut builder, helpers.call, &[interp_ptr, callee, argc_val, argv_ptr]);
                        stack.push(res);
                        guard_error(module, helpers, &mut builder, res, handler);
                    }
                }
                Op::Return => {
                    let ret = stack.pop().unwrap_or_else(|| call0(module, helpers, &mut builder, helpers.nothing, &[]));
                    builder.ins().return_(&[ret]);
                    terminated = true;
                }
                Op::Show => {
                    let v = stack.pop()?;
                    call_void(module, helpers, &mut builder, helpers.show, &[interp_ptr, v]);
                    drop_value(module, helpers, &mut builder, v);
                }
                Op::BuildList(count) => {
                    let n = *count as usize;
                    let mut items = Vec::with_capacity(n);
                    for _ in 0..n {
                        items.push(stack.pop()?);
                    }
                    items.reverse();
                    let (_slot, argv_ptr) = store_in_stack_slot(&mut builder, &items);
                    let count_val = builder.ins().iconst(types::I64, n as i64);
                    let list = call(module, helpers, &mut builder, helpers.build_list, &[count_val, argv_ptr]);
                    stack.push(list);
                    guard_error(module, helpers, &mut builder, list, handler);
                }
                Op::BuildDict(count) => {
                    let n = *count as usize;
                    let mut kv = Vec::with_capacity(n * 2);
                    for _ in 0..n {
                        let value = stack.pop()?;
                        let key = stack.pop()?;
                        kv.push(key);
                        kv.push(value);
                    }
                    let (_slot, kv_ptr) = store_in_stack_slot(&mut builder, &kv);
                    let count_val = builder.ins().iconst(types::I64, n as i64);
                    let dict = call(module, helpers, &mut builder, helpers.build_dict, &[count_val, kv_ptr]);
                    stack.push(dict);
                    guard_error(module, helpers, &mut builder, dict, handler);
                }
                Op::Index => {
                    let idx = stack.pop()?;
                    let obj = stack.pop()?;
                    let res = call(module, helpers, &mut builder, helpers.index_get, &[obj, idx]);
                    stack.push(res);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::IndexSet => {
                    let idx = stack.pop()?;
                    let obj = stack.pop()?;
                    let value = stack.pop()?;
                    let res = call(module, helpers, &mut builder, helpers.index_set, &[obj, idx, value]);
                    guard_error(module, helpers, &mut builder, res, handler);
                    drop_value(module, helpers, &mut builder, res);
                }
                Op::PropertyGet(idx) => {
                    // Transparent-local no-op: the value on the stack is already
                    // the requested field because the preceding LoadLocal loaded
                    // the constructor argument that was stored in its place.
                    let is_transparent = i > 0
                        && func.chunk.ops.get(i - 1).map_or(false, |op| match op {
                            Op::LoadLocal(slot) => transparent_locals
                                .get(slot)
                                .map_or(false, |t| t.field_name_idx == *idx),
                            _ => false,
                        });
                    if is_transparent {
                        // Leave the field value on the stack.
                    } else {
                        let obj = stack.pop()?;
                        let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*idx], *idx);
                        let res = call(module, helpers, &mut builder, helpers.property_get, &[obj, ptr, len]);
                        stack.push(res);
                        guard_error(module, helpers, &mut builder, res, handler);
                    }
                }
                Op::PropertySet(idx) => {
                    let obj = stack.pop()?;
                    let value = stack.pop()?;
                    let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*idx], *idx);
                    let res = call(module, helpers, &mut builder, helpers.property_set, &[obj, ptr, len, value]);
                    guard_error(module, helpers, &mut builder, res, handler);
                    drop_value(module, helpers, &mut builder, res);
                }
                Op::New(arg_count) => {
                    let argc = *arg_count as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(stack.pop()?);
                    }
                    args.reverse();
                    let class = stack.pop()?;
                    // If this `new` feeds a transparent local (a single-field
                    // instance only used to read that field), skip the runtime
                    // allocation and leave the relevant constructor argument on
                    // the stack. The following StoreLocal will save it, and the
                    // PropertyGet becomes a no-op.
                    let transparent_store = func
                        .chunk
                        .ops
                        .get(i + 1)
                        .and_then(|op| match op {
                            Op::StoreLocal(slot) => transparent_locals.get(slot).map(|t| (slot, t)),
                            _ => None,
                        });
                    if let Some((_, t)) = transparent_store {
                        if t.arg_idx < argc {
                            // If the class operand was loaded from a global/local that
                            // we replaced with a placeholder, there is nothing to drop.
                            let class_load_ip = i.saturating_sub(argc + 1);
                            let class_is_placeholder = func
                                .chunk
                                .ops
                                .get(class_load_ip)
                                .map_or(false, |op| matches!(op, Op::LoadLocal(_) | Op::LoadGlobal(_)) && transparent_class_loads.contains(&class_load_ip));
                            if !class_is_placeholder {
                                drop_value(module, helpers, &mut builder, class);
                            }
                            let kept = args[t.arg_idx];
                            for (idx, arg) in args.iter().enumerate() {
                                if idx != t.arg_idx {
                                    drop_value(module, helpers, &mut builder, *arg);
                                }
                            }
                            stack.push(kept);
                        } else {
                            // Fall back to normal allocation if analysis is stale.
                            let (_slot, argv_ptr) = store_in_stack_slot(&mut builder, &args);
                            let argc_val = builder.ins().iconst(types::I64, argc as i64);
                            let res = call(module, helpers, &mut builder, helpers.new_instance, &[interp_ptr, class, argc_val, argv_ptr]);
                            stack.push(res);
                            guard_error(module, helpers, &mut builder, res, handler);
                        }
                    } else {
                        let (_slot, argv_ptr) = store_in_stack_slot(&mut builder, &args);
                        let argc_val = builder.ins().iconst(types::I64, argc as i64);
                        let res = call(module, helpers, &mut builder, helpers.new_instance, &[interp_ptr, class, argc_val, argv_ptr]);
                        stack.push(res);
                        guard_error(module, helpers, &mut builder, res, handler);
                    }
                }
                Op::Tell { name, arg_count } => {
                    let argc = *arg_count as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(stack.pop()?);
                    }
                    args.reverse();
                    let obj = stack.pop()?;
                    let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*name], *name);
                    let (_slot, argv_ptr) = store_in_stack_slot(&mut builder, &args);
                    let argc_val = builder.ins().iconst(types::I64, argc as i64);
                    let res = call(module, helpers, &mut builder, helpers.tell, &[interp_ptr, obj, ptr, len, argc_val, argv_ptr]);
                    stack.push(res);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::BuildClass { name, init, methods, fields, field_init } => {
                    let method_count = methods.len();
                    let mut method_values = Vec::with_capacity(method_count);
                    for _ in 0..method_count {
                        method_values.push(stack.pop()?);
                    }
                    let init_val = if init.is_some() {
                        stack.pop()?
                    } else {
                        builder.ins().iconst(types::I64, 0)
                    };
                    let (_methods_slot, methods_ptr) = store_in_stack_slot(&mut builder, &method_values);
                    let mut method_name_ptrs = Vec::with_capacity(method_count);
                    let mut method_name_lens = Vec::with_capacity(method_count);
                    for m in methods {
                        let (p, l) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*m], *m);
                        method_name_ptrs.push(p);
                        method_name_lens.push(l);
                    }
                    let (_method_names_slot, method_names_ptr) = store_in_stack_slot(&mut builder, &method_name_ptrs);
                    let (_method_lens_slot, method_lens_ptr) = store_in_stack_slot(&mut builder, &method_name_lens);

                    let field_count = fields.len();
                    let mut field_name_ptrs = Vec::with_capacity(field_count);
                    let mut field_name_lens = Vec::with_capacity(field_count);
                    for f in fields {
                        let (p, l) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*f], *f);
                        field_name_ptrs.push(p);
                        field_name_lens.push(l);
                    }
                    let (_field_names_slot, field_names_ptr) = store_in_stack_slot(&mut builder, &field_name_ptrs);
                    let (_field_lens_slot, field_lens_ptr) = store_in_stack_slot(&mut builder, &field_name_lens);
                    let field_init_vals: Vec<ClValue> = field_init.iter().map(|&i| builder.ins().iconst(types::I64, i as i64)).collect();
                    let (_field_init_slot, field_init_ptr) = store_in_stack_slot(&mut builder, &field_init_vals);

                    let (class_name_ptr, class_name_len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*name], *name);
                    let method_count_val = builder.ins().iconst(types::I64, method_count as i64);
                    let field_count_val = builder.ins().iconst(types::I64, field_count as i64);
                    let res = call(module, helpers, &mut builder, helpers.build_class, &[
                        interp_ptr, class_name_ptr, class_name_len, init_val,
                        method_count_val, methods_ptr, method_names_ptr, method_lens_ptr,
                        field_count_val, field_names_ptr, field_lens_ptr, field_init_ptr,
                    ]);
                    stack.push(res);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::IterInit => {
                    let v = stack.pop()?;
                    let res = call1(module, helpers, &mut builder, helpers.iter_init, &[v]);
                    stack.push(res);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::Length => {
                    let v = stack.pop()?;
                    let res = call1(module, helpers, &mut builder, helpers.length, &[v]);
                    stack.push(res);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::TryBegin(catch_ip, catch_slot) => {
                    let catch_block = builder.create_block();
                    let unused = unused_catch_slots.contains(catch_slot);
                    if !unused {
                        builder.append_block_param(catch_block, types::I64);
                    }
                    try_stack.push((catch_block, *catch_slot, *catch_ip));
                    // The try header just falls through to the body.
                    let next = blocks.get(i + 1).copied()?;
                    builder.ins().jump(next, &[]);
                    terminated = true;
                }
                Op::TryEnd => {
                    if let Some(frame) = try_stack.pop() {
                        pending_catches.push(frame);
                    }
                }
                Op::Import(idx) => {
                    let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*idx], *idx);
                    let res = call(module, helpers, &mut builder, helpers.import, &[interp_ptr, ptr, len]);
                    guard_error(module, helpers, &mut builder, res, handler);
                    drop_value(module, helpers, &mut builder, res);
                }
                Op::QualifiedGet(module_idx, name_idx) => {
                    let (mod_ptr, mod_len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*module_idx], *module_idx);
                    let (name_ptr, name_len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*name_idx], *name_idx);
                    let res = call(module, helpers, &mut builder, helpers.qualified_get, &[interp_ptr, mod_ptr, mod_len, name_ptr, name_len]);
                    stack.push(res);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::Export(indices) => {
                    for idx in indices {
                        let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*idx], *idx);
                        let res = call(module, helpers, &mut builder, helpers.export_name, &[interp_ptr, ptr, len]);
                        guard_error(module, helpers, &mut builder, res, handler);
                        drop_value(module, helpers, &mut builder, res);
                    }
                }
                Op::Read => {
                    let path = stack.pop()?;
                    let res = call(module, helpers, &mut builder, helpers.read, &[interp_ptr, path]);
                    stack.push(res);
                    guard_error(module, helpers, &mut builder, res, handler);
                }
                Op::Write => {
                    let path = stack.pop()?;
                    let content = stack.pop()?;
                    let res = call(module, helpers, &mut builder, helpers.write, &[interp_ptr, path, content]);
                    guard_error(module, helpers, &mut builder, res, handler);
                    drop_value(module, helpers, &mut builder, res);
                }
                Op::CheckType(idx) => {
                    let v = *stack.last()?;
                    let (ptr, len) = emit_string(module, helpers, &mut builder, &mut name_strings, &func.chunk.strings[*idx], *idx);
                    let res = call(module, helpers, &mut builder, helpers.check_type, &[interp_ptr, v, ptr, len]);
                    guard_error(module, helpers, &mut builder, res, handler);
                    drop_value(module, helpers, &mut builder, res);
                }
            }

            if !terminated {
                if i + 1 < n {
                    builder.ins().jump(blocks[i + 1], &[]);
                } else {
                    let ret = stack.pop().unwrap_or_else(|| call0(module, helpers, &mut builder, helpers.nothing, &[]));
                    builder.ins().return_(&[ret]);
                }
            }
        }

        builder.switch_to_block(error_block);
        let err = builder.block_params(error_block)[0];
        builder.ins().return_(&[err]);

        for (catch_block, catch_slot, catch_ip) in pending_catches {
            builder.switch_to_block(catch_block);
            if !unused_catch_slots.contains(&catch_slot) {
                let err = builder.block_params(catch_block)[0];
                builder.def_var(locals[catch_slot], err);
            }
            let catch_dest = blocks.get(catch_ip).copied()?;
            builder.ins().jump(catch_dest, &[]);
            builder.seal_block(catch_block);
        }

        for block in &blocks {
            builder.seal_block(*block);
        }
        for block in &extra_blocks {
            builder.seal_block(*block);
        }
        builder.seal_block(error_block);

        builder.finalize();

        let id = module
            .declare_function("jit_generic_main", Linkage::Export, &ctx.func.signature)
            .map_err(|e| if dump { eprintln!("declare function failed: {}", e); })
            .ok()?;
        module.define_function(id, &mut ctx)
            .map_err(|e| if dump { eprintln!("define function failed: {}", e); })
            .ok()?;
        module.clear_context(&mut ctx);
        module.finalize_definitions()
            .map_err(|e| if dump { eprintln!("finalize definitions failed: {}", e); })
            .ok()?;

        let code = module.get_finalized_function(id);
        Some(unsafe { std::mem::transmute::<_, GenericJitFn>(code) })
    }

    fn is_supported(&self, func: &CompiledFunction) -> bool {
        if !func.upvalues.is_empty() {
            return false;
        }
        for op in &func.chunk.ops {
            match op {
                Op::Constant(idx) => match &func.chunk.constants[*idx] {
                    Value::Integer(_)
                    | Value::Number(_)
                    | Value::Bool(_)
                    | Value::String(_)
                    | Value::Nothing => {}
                    _ => return false,
                },
                Op::Nothing
                | Op::True
                | Op::False
                | Op::Pop
                | Op::Dup
                | Op::LoadLocal(_)
                | Op::StoreLocal(_)
                | Op::IncrementLocal(_)
                | Op::AddLocals(_, _)
                | Op::AppendLocalString { .. }
                | Op::AppendLocalList { .. }
                | Op::LoadGlobal(_)
                | Op::StoreGlobal(_)
                | Op::DefineGlobal { .. } => {}
                Op::Closure { .. } => {}
                Op::Binary(_)
                | Op::Unary(_)
                | Op::Jump(_)
                | Op::JumpIfFalse(_)
                | Op::JumpIfTrue(_)
                | Op::Loop(_)
                | Op::Call(_)
                | Op::Return
                | Op::Show
                | Op::BuildList(_)
                | Op::BuildDict(_)
                | Op::Index
                | Op::IndexSet
                | Op::PropertyGet(_)
                | Op::PropertySet(_)
                | Op::New(_)
                | Op::Tell { .. }
                | Op::BuildClass { .. }
                | Op::IterInit
                | Op::Length
                | Op::Import(_)
                | Op::QualifiedGet(_, _)
                | Op::Export(_)
                | Op::Read
                | Op::Write
                | Op::CheckType(_)
                | Op::TryBegin(_, _)
                | Op::TryEnd
                | Op::GetUpvalue(_)
                | Op::SetUpvalue(_) => {}
            }
        }
        for op in &func.chunk.ops {
            match op {
                Op::Jump(t) | Op::JumpIfFalse(t) | Op::JumpIfTrue(t) | Op::Loop(t) => {
                    if *t >= func.chunk.ops.len() {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }
}

fn emit_constant(
    module: &mut cranelift_jit::JITModule,
    helpers: &Helpers,
    builder: &mut FunctionBuilder,
    const_strings: &mut HashMap<usize, cranelift_module::DataId>,
    value: &Value,
    idx: usize,
) -> Option<ClValue> {
    match value {
        Value::Integer(n) => {
            let i = n.to_i64()?;
            let c = builder.ins().iconst(types::I64, i);
            Some(call1(module, helpers, builder, helpers.from_i64, &[c]))
        }
        Value::Number(f) => {
            let c = builder.ins().f64const(*f);
            Some(call1(module, helpers, builder, helpers.from_f64, &[c]))
        }
        Value::Bool(true) => {
            let c = builder.ins().iconst(types::I64, 1);
            Some(call1(module, helpers, builder, helpers.from_bool, &[c]))
        }
        Value::Bool(false) => {
            let c = builder.ins().iconst(types::I64, 0);
            Some(call1(module, helpers, builder, helpers.from_bool, &[c]))
        }
        Value::Nothing => {
            Some(call0(module, helpers, builder, helpers.nothing, &[]))
        }
        Value::String(s) => {
            let id = *const_strings.entry(idx).or_insert_with(|| {
                let name = format!(".strc.{}", idx);
                let data_id = module
                    .declare_data(&name, Linkage::Local, false, false)
                    .expect("declare string data");
                let mut desc = DataDescription::new();
                desc.define(s.as_bytes().to_vec().into());
                module.define_data(data_id, &desc).expect("define string data");
                data_id
            });
            let gv = module.declare_data_in_func(id, builder.func);
            let ptr = builder.ins().global_value(types::I64, gv);
            let len = builder.ins().iconst(types::I64, s.len() as i64);
            Some(call(module, helpers, builder, helpers.from_string, &[ptr, len]))
        }
        _ => None,
    }
}

fn emit_string(
    module: &mut cranelift_jit::JITModule,
    _helpers: &Helpers,
    builder: &mut FunctionBuilder,
    strings: &mut HashMap<usize, cranelift_module::DataId>,
    s: &str,
    idx: usize,
) -> (ClValue, ClValue) {
    let id = *strings.entry(idx).or_insert_with(|| {
        let name = format!(".strn.{}", idx);
        let data_id = module
            .declare_data(&name, Linkage::Local, false, false)
            .expect("declare string data");
        let mut desc = DataDescription::new();
        desc.define(s.as_bytes().to_vec().into());
        module.define_data(data_id, &desc).expect("define string data");
        data_id
    });
    let gv = module.declare_data_in_func(id, builder.func);
    let ptr = builder.ins().global_value(types::I64, gv);
    let len = builder.ins().iconst(types::I64, s.len() as i64);
    (ptr, len)
}

fn store_in_stack_slot(
    builder: &mut FunctionBuilder,
    values: &[ClValue],
) -> (StackSlot, ClValue) {
    let size = (values.len() * 8).max(8) as u32;
    let slot = builder.create_sized_stack_slot(cranelift::codegen::ir::StackSlotData::new(
        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
        size,
        8,
    ));
    for (i, v) in values.iter().enumerate() {
        let offset = (i * 8) as i32;
        builder.ins().stack_store(*v, slot, offset);
    }
    let base = builder.ins().stack_addr(types::I64, slot, 0);
    (slot, base)
}

fn guard_error(
    module: &mut cranelift_jit::JITModule,
    helpers: &Helpers,
    builder: &mut FunctionBuilder,
    value: ClValue,
    handler: Block,
) {
    let callee = module.declare_func_in_func(helpers.is_error, builder.func);
    let call = builder.ins().call(callee, &[value]);
    let flag = builder.inst_results(call)[0];
    let cmp = builder.ins().icmp_imm(IntCC::NotEqual, flag, 0);
    let fallthrough = builder.create_block();
    let handler_has_param = !builder.block_params(handler).is_empty();
    if handler_has_param {
        builder.ins().brif(cmp, handler, &[BlockArg::Value(value)], fallthrough, &[]);
    } else {
        builder.ins().brif(cmp, handler, &[], fallthrough, &[]);
    }
    builder.switch_to_block(fallthrough);
    builder.seal_block(fallthrough);
}

fn set_span(
    module: &mut cranelift_jit::JITModule,
    helpers: &Helpers,
    builder: &mut FunctionBuilder,
    line: i64,
    col: i64,
) {
    let line_val = builder.ins().iconst(types::I64, line);
    let col_val = builder.ins().iconst(types::I64, col);
    call_void(module, helpers, builder, helpers.set_span, &[line_val, col_val]);
}

fn call0(
    module: &mut cranelift_jit::JITModule,
    helpers: &Helpers,
    builder: &mut FunctionBuilder,
    func: cranelift_module::FuncId,
    args: &[ClValue],
) -> ClValue {
    call(module, helpers, builder, func, args)
}

fn call1(
    module: &mut cranelift_jit::JITModule,
    helpers: &Helpers,
    builder: &mut FunctionBuilder,
    func: cranelift_module::FuncId,
    args: &[ClValue],
) -> ClValue {
    call(module, helpers, builder, func, args)
}

fn call(
    module: &mut cranelift_jit::JITModule,
    _helpers: &Helpers,
    builder: &mut FunctionBuilder,
    func: cranelift_module::FuncId,
    args: &[ClValue],
) -> ClValue {
    let callee = module.declare_func_in_func(func, builder.func);
    let call = builder.ins().call(callee, args);
    builder.inst_results(call)[0]
}

fn call_void(
    module: &mut cranelift_jit::JITModule,
    _helpers: &Helpers,
    builder: &mut FunctionBuilder,
    func: cranelift_module::FuncId,
    args: &[ClValue],
) {
    let callee = module.declare_func_in_func(func, builder.func);
    builder.ins().call(callee, args);
}

fn drop_value(
    module: &mut cranelift_jit::JITModule,
    helpers: &Helpers,
    builder: &mut FunctionBuilder,
    value: ClValue,
) {
    call_void(module, helpers, builder, helpers.drop, &[value]);
}

fn declare_helper(
    module: &mut cranelift_jit::JITModule,
    name: &str,
    params: &[Type],
    returns: &[Type],
) -> cranelift_module::FuncId {
    let mut sig = module.make_signature();
    for p in params {
        sig.params.push(AbiParam::new(*p));
    }
    for r in returns {
        sig.returns.push(AbiParam::new(*r));
    }
    module.declare_function(name, Linkage::Import, &sig).unwrap()
}

use std::mem;

use cranelift::prelude::*;
use cranelift_module::{Linkage, Module};

use crate::ast::BinOp;
use crate::bytecode::{CompiledFunction, Op};
use crate::value::Value;

/// A JIT-compiled Period function that takes no arguments and returns an `i64`.
pub type JitFn = unsafe extern "C" fn() -> i64;

extern "C" fn jit_show_i64(value: i64) {
    println!("{}", value);
}

pub struct JitCompiler {
    module: cranelift_jit::JITModule,
    builder_context: FunctionBuilderContext,
    show_func: cranelift_module::FuncId,
}

impl JitCompiler {
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
        let mut builder = cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol("jit_show_i64", jit_show_i64 as *const u8);
        let mut module = cranelift_jit::JITModule::new(builder);

        let mut show_sig = module.make_signature();
        show_sig.params.push(AbiParam::new(types::I64));
        let show_func = module
            .declare_function("jit_show_i64", Linkage::Import, &show_sig)
            .unwrap();

        Self {
            module,
            builder_context: FunctionBuilderContext::new(),
            show_func,
        }
    }

    /// Try to compile a top-level Period function to native code.
    /// Returns `None` if the function contains operations that are not yet
    /// supported by the JIT (callers should fall back to the VM).
    pub fn compile(&mut self, func: &CompiledFunction) -> Option<JitFn> {
        if std::env::var("PERIOD_JIT_DUMP").is_ok() {
            eprintln!("JIT compiling '{}' locals={}", func.name, func.local_count);
            for (i, op) in func.chunk.ops.iter().enumerate() {
                eprintln!("{:3}: {:?}", i, op);
            }
            for (j, f) in func.chunk.functions.iter().enumerate() {
                eprintln!("function {} '{}' locals={}:", j, f.name, f.local_count);
                for (i, op) in f.chunk.ops.iter().enumerate() {
                    eprintln!("  {:3}: {:?}", i, op);
                }
            }
        }
        if !self.is_supported(func) {
            return None;
        }

        let loop_opt = detect_loop_opt(func);

        let mut ctx = self.module.make_context();
        ctx.func.signature.returns.push(AbiParam::new(types::I64));

        let ops = &func.chunk.ops;
        let n = ops.len();

        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut self.builder_context);
        let blocks: Vec<Block> = (0..n).map(|_| builder.create_block()).collect();
        let entry = blocks[0];
        builder.switch_to_block(entry);
        builder.append_block_params_for_function_params(entry);

        // Allocate a Cranelift variable for every local slot and zero-initialise
        // them.  The supported programs use locals as integers only.
        let mut locals: Vec<Variable> = Vec::with_capacity(func.local_count);
        for _ in 0..func.local_count {
            let var = builder.declare_var(types::I64);
            let zero = builder.ins().iconst(types::I64, 0);
            builder.def_var(var, zero);
            locals.push(var);
        }

        let show_callee = self.module.declare_func_in_func(self.show_func, builder.func);

        let mut stack: Vec<cranelift::prelude::Value> = Vec::new();

        for (i, op) in ops.iter().enumerate() {
            let block = blocks[i];
            if builder.current_block() != Some(block) {
                builder.switch_to_block(block);
            }

            // If this op is part of a detected optimisable loop, replace the
            // whole loop with its result and jump over the body.
            if let Some(opt) = &loop_opt {
                match opt {
                    LoopOpt::Series(s) => {
                        if i >= s.cond_start && i <= s.loop_end {
                            let exit_block = blocks.get(s.exit).copied()?;
                            if i == s.cond_start {
                                let cur_acc = builder.use_var(locals[s.acc_slot]);
                                let inc = builder.ins().iconst(types::I64, s.acc_sum);
                                let new_acc = builder.ins().iadd(cur_acc, inc);
                                builder.def_var(locals[s.acc_slot], new_acc);
                                let final_i = builder.ins().iconst(types::I64, s.final_i);
                                builder.def_var(locals[s.i_slot], final_i);
                                builder.ins().jump(exit_block, &[]);
                            } else {
                                builder.ins().jump(exit_block, &[]);
                            }
                            continue;
                        }
                    }
                    LoopOpt::ModularCount(c) => {
                        if i >= c.cond_start && i <= c.loop_end {
                            let exit_block = blocks.get(c.exit).copied()?;
                            if i == c.cond_start {
                                let cur_acc = builder.use_var(locals[c.acc_slot]);
                                let inc = builder.ins().iconst(types::I64, c.acc_inc);
                                let new_acc = builder.ins().iadd(cur_acc, inc);
                                builder.def_var(locals[c.acc_slot], new_acc);
                                let final_i = builder.ins().iconst(types::I64, c.final_i);
                                builder.def_var(locals[c.i_slot], final_i);
                                builder.ins().jump(exit_block, &[]);
                            } else {
                                builder.ins().jump(exit_block, &[]);
                            }
                            continue;
                        }
                    }
                    LoopOpt::Evaluated { cond_start, loop_end, exit, final_values } => {
                        if i >= *cond_start && i <= *loop_end {
                            let exit_block = blocks.get(*exit).copied()?;
                            if i == *cond_start {
                                for (slot, val) in final_values.iter().enumerate() {
                                    if let Some(v) = val {
                                        let c = builder.ins().iconst(types::I64, *v);
                                        builder.def_var(locals[slot], c);
                                    }
                                }
                                builder.ins().jump(exit_block, &[]);
                            } else {
                                builder.ins().jump(exit_block, &[]);
                            }
                            continue;
                        }
                    }
                }
            }

            let mut terminated = false;
            match op {
                Op::Constant(idx) => {
                    let value = &func.chunk.constants[*idx];
                    let n = value_to_i64(value)?;
                    let v = builder.ins().iconst(types::I64, n);
                    stack.push(v);
                }
                Op::Nothing => {
                    stack.push(builder.ins().iconst(types::I64, 0));
                }
                Op::True => {
                    stack.push(builder.ins().iconst(types::I64, 1));
                }
                Op::False => {
                    stack.push(builder.ins().iconst(types::I64, 0));
                }
                Op::Pop => {
                    stack.pop()?;
                }
                Op::Dup => {
                    let v = *stack.last()?;
                    stack.push(v);
                }
                Op::LoadLocal(slot) => {
                    let v = builder.use_var(locals[*slot]);
                    stack.push(v);
                }
                Op::StoreLocal(slot) => {
                    let v = stack.pop()?;
                    builder.def_var(locals[*slot], v);
                }
                Op::IncrementLocal(slot) => {
                    let cur = builder.use_var(locals[*slot]);
                    let one = builder.ins().iconst(types::I64, 1);
                    let next = builder.ins().iadd(cur, one);
                    builder.def_var(locals[*slot], next);
                }
                Op::AddLocals(target, source) => {
                    let cur = builder.use_var(locals[*target]);
                    let src = builder.use_var(locals[*source]);
                    let next = builder.ins().iadd(cur, src);
                    builder.def_var(locals[*target], next);
                }
                Op::Binary(bin_op) => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let result = translate_binary(&mut builder, *bin_op, left, right)?;
                    stack.push(result);
                }
                Op::JumpIfFalse(target) => {
                    let cond = stack.pop()?;
                    let cmp = builder.ins().icmp_imm(IntCC::NotEqual, cond, 0);
                    let fallthrough = blocks.get(i + 1).copied()?;
                    let dest = blocks.get(*target).copied()?;
                    builder.ins().brif(cmp, fallthrough, &[], dest, &[]);
                    terminated = true;
                }
                Op::JumpIfTrue(target) => {
                    let cond = stack.pop()?;
                    let cmp = builder.ins().icmp_imm(IntCC::NotEqual, cond, 0);
                    let dest = blocks.get(*target).copied()?;
                    let fallthrough = blocks.get(i + 1).copied()?;
                    builder.ins().brif(cmp, dest, &[], fallthrough, &[]);
                    terminated = true;
                }
                Op::Jump(target) | Op::Loop(target) => {
                    let dest = blocks.get(*target).copied()?;
                    builder.ins().jump(dest, &[]);
                    terminated = true;
                }
                Op::Show => {
                    let v = stack.pop()?;
                    builder.ins().call(show_callee, &[v]);
                }
                Op::Return => {
                    let ret = stack.pop().unwrap_or_else(|| builder.ins().iconst(types::I64, 0));
                    builder.ins().return_(&[ret]);
                    terminated = true;
                }
                _ => return None,
            }

            if !terminated {
                if i + 1 < n {
                    builder.ins().jump(blocks[i + 1], &[]);
                } else {
                    let zero = builder.ins().iconst(types::I64, 0);
                    builder.ins().return_(&[zero]);
                }
            }
        }

        for block in &blocks {
            builder.seal_block(*block);
        }

        builder.finalize();

        let id = self
            .module
            .declare_function("jit_main", Linkage::Export, &ctx.func.signature)
            .ok()?;
        self.module.define_function(id, &mut ctx).ok()?;
        self.module.clear_context(&mut ctx);
        self.module.finalize_definitions().ok()?;

        let code = self.module.get_finalized_function(id);
        Some(unsafe { mem::transmute::<_, JitFn>(code) })
    }

    fn is_supported(&self, func: &CompiledFunction) -> bool {
        // Only compile functions without upvalues/captures and whose bytecode is
        // composed entirely of integer operations we can translate.
        if !func.upvalues.is_empty() {
            return false;
        }
        let ops = &func.chunk.ops;
        for op in ops {
            match op {
                Op::Constant(idx) => {
                    if value_to_i64(&func.chunk.constants[*idx]).is_none() {
                        return false;
                    }
                }
                Op::Nothing
                | Op::True
                | Op::False
                | Op::Pop
                | Op::Dup
                | Op::LoadLocal(_)
                | Op::StoreLocal(_)
                | Op::IncrementLocal(_)
                | Op::AddLocals(_, _)
                | Op::Binary(_)
                | Op::Jump(_)
                | Op::JumpIfFalse(_)
                | Op::JumpIfTrue(_)
                | Op::Loop(_)
                | Op::Show
                | Op::Return => {}
                _ => return false,
            }
        }
        // Targets must be in range.
        for op in ops {
            match op {
                Op::Jump(t) | Op::JumpIfFalse(t) | Op::Loop(t) => {
                    if *t >= ops.len() {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }
}

enum LoopOpt {
    Series(SeriesLoop),
    ModularCount(ModularCountLoop),
    Evaluated {
        cond_start: usize,
        loop_end: usize,
        exit: usize,
        final_values: Vec<Option<i64>>,
    },
}

/// Detect loops that can be optimised away:
///   1. Closed-form arithmetic series.
///   2. Side-effect-free loops with a constant bound that can be evaluated at
///      compile time.
fn detect_loop_opt(func: &CompiledFunction) -> Option<LoopOpt> {
    let ops = &func.chunk.ops;
    for (idx, op) in ops.iter().enumerate() {
        let Op::Loop(target) = op else { continue };
        let cond_start = *target;
        if cond_start + 4 > ops.len() || idx < cond_start + 4 {
            continue;
        }
        let Op::LoadLocal(i_slot) = &ops[cond_start] else { continue };
        let Op::Constant(n_idx) = &ops[cond_start + 1] else { continue };
        let cond_op = match &ops[cond_start + 2] {
            Op::Binary(op) => *op,
            _ => continue,
        };
        if !matches!(cond_op, BinOp::Lt | BinOp::Le) {
            continue;
        }
        let Op::JumpIfFalse(exit) = &ops[cond_start + 3] else { continue };
        let exit = *exit;
        if exit <= idx {
            continue;
        }
        let n = value_to_i64(&func.chunk.constants[*n_idx])?;
        if n < 0 {
            continue;
        }
        let slot_consts = prefix_constant_state(func, cond_start)?;
        let init_i = match slot_consts[*i_slot] {
            Some(v) => v,
            None => continue,
        };
        let iter_count = match cond_op {
            BinOp::Lt => n.checked_sub(init_i).filter(|&m| m > 0)?,
            BinOp::Le => n.checked_sub(init_i).and_then(|m| m.checked_add(1)).filter(|&m| m > 0)?,
            _ => continue,
        };

        if let Some(s) = try_series_opt(func, cond_start, idx, exit, *i_slot, cond_op, n, init_i, iter_count, &slot_consts) {
            return Some(LoopOpt::Series(s));
        }

        if let Some(c) = try_modular_count_opt(func, cond_start, idx, exit, *i_slot, n, init_i, iter_count, &slot_consts) {
            return Some(LoopOpt::ModularCount(c));
        }

        const EVAL_CAP: usize = 10_000_000;
        if iter_count as usize <= EVAL_CAP {
            if let Some(final_values) = evaluate_loop(func, cond_start, idx, *i_slot, iter_count, &slot_consts) {
                return Some(LoopOpt::Evaluated { cond_start, loop_end: idx, exit, final_values });
            }
        }
    }
    None
}

struct SeriesLoop {
    cond_start: usize,
    loop_end: usize,
    exit: usize,
    i_slot: usize,
    acc_slot: usize,
    acc_sum: i64,
    final_i: i64,
}

/// Closed-form optimisation for `acc += i` loops where `i` starts at a known
/// constant and increments by 1 each iteration.
fn try_series_opt(
    func: &CompiledFunction,
    cond_start: usize,
    loop_end: usize,
    exit: usize,
    i_slot: usize,
    cond_op: BinOp,
    n: i64,
    init_i: i64,
    iter_count: i64,
    slot_consts: &[Option<i64>],
) -> Option<SeriesLoop> {
    let ops = &func.chunk.ops;
    if loop_end < cond_start + 6 || loop_end - 2 != cond_start + 4 {
        return None;
    }
    let Op::AddLocals(acc_slot, src) = &ops[loop_end - 2] else { return None };
    if *src != i_slot {
        return None;
    }
    let Op::IncrementLocal(inc_slot) = &ops[loop_end - 1] else { return None };
    if *inc_slot != i_slot {
        return None;
    }
    let init_acc = slot_consts[*acc_slot]?;
    let a = init_i;
    let m = iter_count as i128;
    // sum_i = a + (a+1) + ... + (a+m-1) = m*a + m*(m-1)/2
    let sum_i = m.checked_mul(a as i128)?
        .checked_add(m.checked_mul(m.checked_sub(1)?)?.checked_div(2)?)?;
    let acc_sum_128 = sum_i.checked_add(init_acc as i128)?;
    if acc_sum_128 < i64::MIN as i128 || acc_sum_128 > i64::MAX as i128 {
        return None;
    }
    let final_i = match cond_op {
        BinOp::Lt => init_i.checked_add(iter_count)?,
        BinOp::Le => n.checked_add(1)?,
        _ => return None,
    };
    Some(SeriesLoop {
        cond_start,
        loop_end,
        exit,
        i_slot,
        acc_slot: *acc_slot,
        acc_sum: acc_sum_128 as i64,
        final_i,
    })
}

/// Compile-time evaluation of a simple side-effect-free integer loop.
/// The loop body may contain conditional jumps (e.g. short-circuit `and`/`or`)
/// but no nested loops or side-effecting operations.
/// Returns the final known values of all local slots after the loop.
struct ModularCountLoop {
    cond_start: usize,
    loop_end: usize,
    exit: usize,
    i_slot: usize,
    acc_slot: usize,
    acc_inc: i64,
    final_i: i64,
}

/// Closed-form optimisation for loops that count how many iterations satisfy a
/// predicate built only from `i % d == c` or `i % d != c` tests.  Because the
/// predicate is periodic with period `lcm(d1, d2, ...)`, we evaluate a single
/// period at compile time and scale it to the full iteration count.
fn try_modular_count_opt(
    func: &CompiledFunction,
    cond_start: usize,
    loop_end: usize,
    exit: usize,
    i_slot: usize,
    n: i64,
    init_i: i64,
    iter_count: i64,
    slot_consts: &[Option<i64>],
) -> Option<ModularCountLoop> {
    let ops = &func.chunk.ops;
    let body_start = cond_start + 4;
    let body_end = loop_end;
    if body_start >= body_end {
        return None;
    }

    // Find the induction-variable increment and a single counter increment.
    let mut inc_i_idx = None;
    let mut inc_acc: Option<(usize, usize)> = None;
    for ip in body_start..body_end {
        match &ops[ip] {
            Op::IncrementLocal(slot) => {
                if *slot == i_slot {
                    inc_i_idx = Some(ip);
                } else if inc_acc.is_none() {
                    inc_acc = Some((ip, *slot));
                } else {
                    return None;
                }
            }
            Op::StoreLocal(_) | Op::AddLocals(_, _) => return None,
            Op::Show | Op::Return => return None,
            _ => {}
        }
    }
    let _inc_i_idx = inc_i_idx?;
    let (_, acc_slot) = inc_acc?;

    // The predicate may only inspect `i` through modulo operations.
    let mut moduli: Vec<usize> = Vec::new();
    for ip in body_start..body_end {
        if let Op::LoadLocal(slot) = &ops[ip] {
            if *slot != i_slot {
                return None;
            }
            // Expect `LoadLocal(i), Constant(d), Binary(Mod)`.
            let d = match ops.get(ip + 1).and_then(|op| match op {
                Op::Constant(idx) => value_to_i64(&func.chunk.constants[*idx]),
                _ => None,
            }) {
                Some(d) if d > 0 => d as usize,
                _ => return None,
            };
            if !matches!(ops.get(ip + 2), Some(Op::Binary(BinOp::Mod))) {
                return None;
            }
            moduli.push(d);
        }
    }
    if moduli.is_empty() {
        return None;
    }

    const LCM_CAP: usize = 100_000;
    let mut period = 1usize;
    for d in &moduli {
        period = lcm(period, *d);
        if period > LCM_CAP {
            return None;
        }
    }

    // Simulate one full period to count how many times the counter increments.
    let mut period_init = slot_consts.to_vec();
    period_init[i_slot] = Some(init_i);
    period_init[acc_slot] = Some(0);
    let after_period = evaluate_loop(func, cond_start, loop_end, i_slot, period as i64, &period_init)?;
    let per_period = after_period.get(acc_slot).copied().flatten()?;

    let full = (iter_count as usize) / period;
    let rem = (iter_count as usize) % period;
    let mut total_inc_128 = (per_period as i128).checked_mul(full as i128)?;

    if rem > 0 {
        let start_i = (init_i as i128).checked_add((full * period) as i128)?;
        if start_i < i64::MIN as i128 || start_i > i64::MAX as i128 {
            return None;
        }
        let mut rem_init = slot_consts.to_vec();
        rem_init[i_slot] = Some(start_i as i64);
        rem_init[acc_slot] = Some(0);
        let after_rem = evaluate_loop(func, cond_start, loop_end, i_slot, rem as i64, &rem_init)?;
        let rem_inc = after_rem.get(acc_slot).copied().flatten()?;
        total_inc_128 = total_inc_128.checked_add(rem_inc as i128)?;
    }

    let init_acc = slot_consts[acc_slot].unwrap_or(0);
    let final_acc_128 = (init_acc as i128).checked_add(total_inc_128)?;
    if final_acc_128 < i64::MIN as i128 || final_acc_128 > i64::MAX as i128 {
        return None;
    }
    let final_i = n.checked_add(1)?;
    Some(ModularCountLoop {
        cond_start,
        loop_end,
        exit,
        i_slot,
        acc_slot,
        acc_inc: final_acc_128 as i64 - init_acc,
        final_i,
    })
}

/// Compile-time evaluation of a simple side-effect-free integer loop.
/// The loop body may contain conditional jumps (e.g. short-circuit `and`/`or`)
/// but no side-effecting operations.
/// Returns the final known values of all local slots after the loop.
fn evaluate_loop(
    func: &CompiledFunction,
    cond_start: usize,
    loop_end: usize,
    i_slot: usize,
    n: i64,
    init: &[Option<i64>],
) -> Option<Vec<Option<i64>>> {
    let ops = &func.chunk.ops;
    let body_start = cond_start + 4;
    let body_end = loop_end;
    if body_start >= body_end {
        return None;
    }

    // Verify every body op is supported and all jumps stay inside the body.
    for (_ip, op) in ops.iter().enumerate().take(body_end).skip(body_start) {
        match op {
            Op::Constant(_)
            | Op::Nothing
            | Op::True
            | Op::False
            | Op::Pop
            | Op::Dup
            | Op::LoadLocal(_)
            | Op::StoreLocal(_)
            | Op::IncrementLocal(_)
            | Op::AddLocals(_, _) => {}
            Op::Jump(target) | Op::JumpIfFalse(target) | Op::JumpIfTrue(target) => {
                if *target < body_start || *target > body_end || *target == body_end {
                    return None;
                }
            }
            Op::Binary(bin_op) => match bin_op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Mod | BinOp::Lt | BinOp::Gt
                | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne => {}
                _ => return None,
            },
            _ => return None,
        }
    }

    // The induction variable must be incremented somewhere in the body.
    let mut increments_i = false;
    for op in &ops[body_start..body_end] {
        if let Op::IncrementLocal(s) = op {
            if *s == i_slot {
                increments_i = true;
            }
        }
    }
    if !increments_i {
        return None;
    }

    let mut slots: Vec<Option<i64>> = init.to_vec();
    let mut stack: Vec<Option<i64>> = Vec::new();
    let n = n as usize;

    for _ in 0..n {
        let mut ip = body_start;
        while ip < body_end {
            let op = &ops[ip];
            match op {
                Op::Constant(idx) => stack.push(value_to_i64(&func.chunk.constants[*idx])),
                Op::Nothing => stack.push(Some(0)),
                Op::True => stack.push(Some(1)),
                Op::False => stack.push(Some(0)),
                Op::Pop => {
                    stack.pop()?;
                }
                Op::Dup => {
                    let v = *stack.last()?;
                    stack.push(v);
                }
                Op::LoadLocal(slot) => stack.push(slots[*slot]),
                Op::StoreLocal(slot) => {
                    slots[*slot] = stack.pop()?;
                }
                Op::IncrementLocal(slot) => {
                    slots[*slot] = slots[*slot].and_then(|x| x.checked_add(1));
                }
                Op::AddLocals(target, source) => {
                    slots[*target] = match (slots[*target], slots[*source]) {
                        (Some(a), Some(b)) => a.checked_add(b),
                        _ => None,
                    };
                }
                Op::Jump(target) => {
                    ip = *target;
                    continue;
                }
                Op::JumpIfFalse(target) => {
                    let cond = stack.pop().flatten()?;
                    if cond == 0 {
                        ip = *target;
                        continue;
                    }
                }
                Op::JumpIfTrue(target) => {
                    let cond = stack.pop().flatten()?;
                    if cond != 0 {
                        ip = *target;
                        continue;
                    }
                }
                Op::Binary(bin_op) => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let res = match bin_op {
                        BinOp::Add => left.zip(right).and_then(|(a, b)| a.checked_add(b)),
                        BinOp::Sub => left.zip(right).and_then(|(a, b)| a.checked_sub(b)),
                        BinOp::Mul => left.zip(right).and_then(|(a, b)| a.checked_mul(b)),
                        BinOp::Mod => left.zip(right).and_then(|(a, b)| {
                            if b == 0 { None } else { a.checked_rem(b) }
                        }),
                        BinOp::Lt => left.zip(right).map(|(a, b)| if a < b { 1 } else { 0 }),
                        BinOp::Gt => left.zip(right).map(|(a, b)| if a > b { 1 } else { 0 }),
                        BinOp::Le => left.zip(right).map(|(a, b)| if a <= b { 1 } else { 0 }),
                        BinOp::Ge => left.zip(right).map(|(a, b)| if a >= b { 1 } else { 0 }),
                        BinOp::Eq => left.zip(right).map(|(a, b)| if a == b { 1 } else { 0 }),
                        BinOp::Ne => left.zip(right).map(|(a, b)| if a != b { 1 } else { 0 }),
                        _ => return None,
                    };
                    stack.push(res);
                }
                _ => return None,
            }
            ip += 1;
        }
    }

    Some(slots)
}

/// Simple constant propagation for the straight-line prefix before a loop.
/// Returns the constant value (if known) of each local slot at the loop entry.
fn prefix_constant_state(func: &CompiledFunction, end: usize) -> Option<Vec<Option<i64>>> {
    let mut stack: Vec<Option<i64>> = Vec::new();
    let mut slot_consts: Vec<Option<i64>> = vec![None; func.local_count];
    for op in &func.chunk.ops[..end] {
        match op {
            Op::Constant(idx) => stack.push(value_to_i64(&func.chunk.constants[*idx])),
            Op::Nothing => stack.push(Some(0)),
            Op::True => stack.push(Some(1)),
            Op::False => stack.push(Some(0)),
            Op::Pop => {
                stack.pop()?;
            }
            Op::Dup => {
                let v = *stack.last()?;
                stack.push(v);
            }
            Op::LoadLocal(slot) => stack.push(slot_consts[*slot]),
            Op::StoreLocal(slot) => {
                let v = stack.pop()?;
                slot_consts[*slot] = v;
            }
            Op::IncrementLocal(slot) | Op::AddLocals(slot, _) => {
                slot_consts[*slot] = None;
            }
            Op::Binary(_) => {
                stack.pop()?;
                stack.pop()?;
                stack.push(None);
            }
            Op::Jump(_) | Op::JumpIfFalse(_) | Op::Loop(_) | Op::Show | Op::Return => {
                return None;
            }
            _ => return None,
        }
    }
    Some(slot_consts)
}

fn gcd(a: usize, b: usize) -> usize {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

fn lcm(a: usize, b: usize) -> usize {
    a / gcd(a, b) * b
}

fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Integer(n) => n.to_i64(),
        _ => None,
    }
}

fn translate_binary(
    builder: &mut FunctionBuilder,
    op: BinOp,
    left: cranelift::prelude::Value,
    right: cranelift::prelude::Value,
) -> Option<cranelift::prelude::Value> {
    let v = match op {
        BinOp::Add => builder.ins().iadd(left, right),
        BinOp::Sub => builder.ins().isub(left, right),
        BinOp::Mul => builder.ins().imul(left, right),
        BinOp::Mod => builder.ins().srem(left, right),
        BinOp::Div => {
            // Period integer division returns a number; keep it unsupported for
            // now to avoid accidentally changing semantics in the JIT path.
            return None;
        }
        BinOp::Lt => builder.ins().icmp(IntCC::SignedLessThan, left, right),
        BinOp::Gt => builder.ins().icmp(IntCC::SignedGreaterThan, left, right),
        BinOp::Le => builder.ins().icmp(IntCC::SignedLessThanOrEqual, left, right),
        BinOp::Ge => builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, left, right),
        BinOp::Eq => builder.ins().icmp(IntCC::Equal, left, right),
        BinOp::Ne => builder.ins().icmp(IntCC::NotEqual, left, right),
        _ => return None,
    };

    // Cranelift comparisons return an i1/i8 value.  Normalise to i64 so that
    // the value stack is homogeneous and JumpIfFalse can test against zero.
    if builder.func.dfg.value_type(v) == types::I8 {
        Some(builder.ins().uextend(types::I64, v))
    } else {
        Some(v)
    }
}

/// Try to execute a program entirely at compile time.  This is used as a fast
/// path for simple numeric programs whose loops can be replaced by closed-form
/// or periodic optimisations, avoiding the overhead of Cranelift codegen.
pub fn try_run_constant(func: &CompiledFunction) -> Option<()> {
    // All operations must be supported by our i64 interpreter.
    for op in &func.chunk.ops {
        match op {
            Op::Constant(idx) => {
                if value_to_i64(&func.chunk.constants[*idx]).is_none() {
                    return None;
                }
            }
            Op::Nothing
            | Op::True
            | Op::False
            | Op::Pop
            | Op::Dup
            | Op::LoadLocal(_)
            | Op::StoreLocal(_)
            | Op::IncrementLocal(_)
            | Op::AddLocals(_, _)
            | Op::Jump(_)
            | Op::JumpIfFalse(_)
            | Op::JumpIfTrue(_)
            | Op::Loop(_)
            | Op::Show
            | Op::Return => {}
            Op::Binary(bin_op) => match bin_op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Mod
                | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {}
                _ => return None,
            },
            _ => return None,
        }
    }

    let loop_count = func.chunk.ops.iter().filter(|op| matches!(op, Op::Loop(_))).count();
    let loop_opt = detect_loop_opt(func);
    if loop_count > 1 {
        // Conservative: only handle single-loop programs here.
        return None;
    }
    if loop_count == 1 && loop_opt.is_none() {
        return None;
    }

    let mut locals: Vec<Option<i64>> = vec![Some(0); func.local_count];
    let mut stack: Vec<Option<i64>> = Vec::new();
    let mut ip: usize = 0;

    while ip < func.chunk.ops.len() {
        if let Some(ref opt) = loop_opt {
            let cond_start = match opt {
                LoopOpt::Series(s) => s.cond_start,
                LoopOpt::ModularCount(c) => c.cond_start,
                LoopOpt::Evaluated { cond_start, .. } => *cond_start,
            };
            if ip == cond_start {
                match opt {
                    LoopOpt::Series(s) => {
                        locals[s.acc_slot] = locals[s.acc_slot]
                            .zip(Some(s.acc_sum))
                            .and_then(|(a, b)| a.checked_add(b));
                        locals[s.i_slot] = Some(s.final_i);
                        ip = s.exit;
                    }
                    LoopOpt::ModularCount(c) => {
                        locals[c.acc_slot] = locals[c.acc_slot]
                            .zip(Some(c.acc_inc))
                            .and_then(|(a, b)| a.checked_add(b));
                        locals[c.i_slot] = Some(c.final_i);
                        ip = c.exit;
                    }
                    LoopOpt::Evaluated { exit, final_values, .. } => {
                        for (slot, val) in final_values.iter().enumerate() {
                            if let Some(v) = val {
                                locals[slot] = Some(*v);
                            }
                        }
                        ip = *exit;
                    }
                }
                continue;
            }
        }

        let op = &func.chunk.ops[ip];
        let mut next_ip = ip + 1;
        match op {
            Op::Constant(idx) => stack.push(value_to_i64(&func.chunk.constants[*idx])),
            Op::Nothing => stack.push(Some(0)),
            Op::True => stack.push(Some(1)),
            Op::False => stack.push(Some(0)),
            Op::Pop => {
                stack.pop()?;
            }
            Op::Dup => {
                let v = *stack.last()?;
                stack.push(v);
            }
            Op::LoadLocal(slot) => stack.push(locals[*slot]),
            Op::StoreLocal(slot) => {
                locals[*slot] = stack.pop()?;
            }
            Op::IncrementLocal(slot) => {
                locals[*slot] = locals[*slot].and_then(|x| x.checked_add(1));
            }
            Op::AddLocals(target, source) => {
                locals[*target] = match (locals[*target], locals[*source]) {
                    (Some(a), Some(b)) => a.checked_add(b),
                    _ => None,
                };
            }
            Op::Binary(bin_op) => {
                let right = stack.pop()?;
                let left = stack.pop()?;
                let res = match bin_op {
                    BinOp::Add => left.zip(right).and_then(|(a, b)| a.checked_add(b)),
                    BinOp::Sub => left.zip(right).and_then(|(a, b)| a.checked_sub(b)),
                    BinOp::Mul => left.zip(right).and_then(|(a, b)| a.checked_mul(b)),
                    BinOp::Mod => left.zip(right).and_then(|(a, b)| {
                        if b == 0 { None } else { a.checked_rem(b) }
                    }),
                    BinOp::Lt => left.zip(right).map(|(a, b)| if a < b { 1 } else { 0 }),
                    BinOp::Gt => left.zip(right).map(|(a, b)| if a > b { 1 } else { 0 }),
                    BinOp::Le => left.zip(right).map(|(a, b)| if a <= b { 1 } else { 0 }),
                    BinOp::Ge => left.zip(right).map(|(a, b)| if a >= b { 1 } else { 0 }),
                    BinOp::Eq => left.zip(right).map(|(a, b)| if a == b { 1 } else { 0 }),
                    BinOp::Ne => left.zip(right).map(|(a, b)| if a != b { 1 } else { 0 }),
                    _ => return None,
                };
                stack.push(res);
            }
            Op::Jump(target) | Op::Loop(target) => {
                next_ip = *target;
            }
            Op::JumpIfFalse(target) => {
                let cond = stack.pop().flatten()?;
                if cond == 0 {
                    next_ip = *target;
                }
            }
            Op::JumpIfTrue(target) => {
                let cond = stack.pop().flatten()?;
                if cond != 0 {
                    next_ip = *target;
                }
            }
            Op::Show => {
                let v = stack.pop().flatten()?;
                println!("{}", v);
            }
            Op::Return => {
                return Some(());
            }
            _ => return None,
        }
        ip = next_ip;
    }

    Some(())
}

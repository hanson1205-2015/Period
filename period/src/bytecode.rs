use std::rc::Rc;

use crate::ast::{BinOp, Span, UnaryOp};
use crate::value::Value;

#[derive(Clone, Debug)]
pub enum Op {
    Constant(usize),
    Nothing,
    True,
    False,
    Pop,
    Dup,
    LoadLocal(usize),
    StoreLocal(usize),
    LoadGlobal(usize),
    StoreGlobal(usize),
    DefineGlobal { name: usize, type_ann: Option<usize> },
    Closure { func: usize, upvalues: Vec<Upvalue> },
    GetUpvalue(usize),
    SetUpvalue(usize),
    Binary(BinOp),
    Unary(UnaryOp),
    Jump(usize),
    JumpIfFalse(usize),
    JumpIfTrue(usize),
    Loop(usize),
    Call(u8),
    Return,
    Show,
    BuildList(usize),
    BuildDict(usize),
    Index,
    IndexSet,
    PropertyGet(usize),
    PropertySet(usize),
    New(u8),
    Tell { name: usize, arg_count: u8 },
    BuildClass { name: usize, init: Option<usize>, methods: Vec<usize>, fields: Vec<usize>, field_init: Vec<usize> },
    IterInit,
    Length,
    TryBegin(usize, usize),
    TryEnd,
    Import(usize),
    QualifiedGet(usize, usize),
    Export(Vec<usize>),
    Read,
    Write,
    CheckType(usize),
    IncrementLocal(usize),
    AddLocals(usize, usize),
    AppendLocalString { slot: usize, string_idx: usize },
    AppendLocalList { slot: usize },
}

#[derive(Clone, Debug)]
pub struct Upvalue {
    pub is_local: bool,
    pub index: usize,
}

#[derive(Clone, Default)]
pub struct Chunk {
    pub ops: Vec<Op>,
    pub spans: Vec<Span>,
    pub constants: Vec<Value>,
    pub strings: Vec<String>,
    pub functions: Vec<Rc<CompiledFunction>>,
}

impl Chunk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn emit(&mut self, op: Op, span: Span) -> usize {
        let idx = self.ops.len();
        self.ops.push(op);
        self.spans.push(span);
        idx
    }

    pub fn add_constant(&mut self, value: Value) -> usize {
        let idx = self.constants.len();
        self.constants.push(value);
        idx
    }

    pub fn add_string(&mut self, s: String) -> usize {
        let idx = self.strings.len();
        self.strings.push(s);
        idx
    }

    pub fn add_function(&mut self, func: CompiledFunction) -> usize {
        let idx = self.functions.len();
        self.functions.push(Rc::new(func));
        idx
    }

    pub fn patch_jump(&mut self, idx: usize) {
        let target = self.ops.len();
        match &mut self.ops[idx] {
            Op::Jump(t) | Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => {
                *t = target;
            }
            _ => panic!("patch_jump on non-jump instruction"),
        }
    }

    pub fn patch_try_begin(&mut self, idx: usize) {
        let target = self.ops.len();
        match &mut self.ops[idx] {
            Op::TryBegin(t, _) => {
                *t = target;
            }
            _ => panic!("patch_try_begin on non-try instruction"),
        }
    }
}

#[derive(Clone)]
pub struct CompiledFunction {
    pub name: String,
    pub params: Vec<(String, Option<String>)>,
    pub return_type: Option<String>,
    pub chunk: Chunk,
    pub local_count: usize,
    pub upvalues: Vec<Upvalue>,
    pub span: Span,
}

impl CompiledFunction {
    pub fn new(name: impl Into<String>, params: Vec<(String, Option<String>)>, return_type: Option<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            params,
            return_type,
            chunk: Chunk::new(),
            local_count: 0,
            upvalues: Vec::new(),
            span,
        }
    }
}

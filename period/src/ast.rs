use std::fmt;

use num_bigint::BigInt;

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Integer(BigInt, Span),
    Number(f64, Span),
    String(String, Span),
    Bool(bool, Span),
    Nothing(Span),
    Variable { name: String, span: Span },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr>, span: Span },
    Unary { op: UnaryOp, operand: Box<Expr>, span: Span },
    Call { callee: Box<Expr>, args: Vec<Expr>, span: Span },
    Index { object: Box<Expr>, index: Box<Expr>, span: Span },
    Property { object: Box<Expr>, name: String, span: Span },
    New { class: Box<Expr>, args: Vec<Expr>, span: Span },
    Tell { object: Box<Expr>, method: String, args: Vec<Expr>, span: Span },
    Qualified { name: String, module: String, span: Span },
    List(Vec<Expr>, Span),
    Dict(Vec<(Expr, Expr)>, Span),
    Ellipsis,
}

impl Expr {
    /// Return the source span attached to this expression, if any.
    pub fn span(&self) -> Option<&Span> {
        match self {
            Expr::Integer(_, span)
            | Expr::Number(_, span)
            | Expr::String(_, span)
            | Expr::Bool(_, span)
            | Expr::Nothing(span)
            | Expr::Variable { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Call { span, .. }
            | Expr::Index { span, .. }
            | Expr::Property { span, .. }
            | Expr::New { span, .. }
            | Expr::Tell { span, .. }
            | Expr::Qualified { span, .. } => Some(span),
            Expr::List(_, span) | Expr::Dict(_, span) => Some(span),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod, Pow,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*", BinOp::Div => "/",
            BinOp::Mod => "%", BinOp::Pow => "**",
            BinOp::Eq => "==", BinOp::Ne => "!=", BinOp::Lt => "<", BinOp::Gt => ">",
            BinOp::Le => "<=", BinOp::Ge => ">=",
            BinOp::And => "and", BinOp::Or => "or",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UnaryOp {
    Neg, Not,
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnaryOp::Neg => write!(f, "-"),
            UnaryOp::Not => write!(f, "not "),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AssignTarget {
    Variable { name: String, span: Span },
    Index { object: Box<Expr>, index: Box<Expr>, span: Span },
    Property { object: Box<Expr>, name: String, span: Span },
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let { name: String, type_ann: Option<String>, value: Expr, span: Span },
    Set { target: AssignTarget, value: Expr },
    Show(Expr),
    If { cond: Expr, then_branch: Vec<Stmt>, else_branch: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    For { var: String, iterable: Expr, body: Vec<Stmt> },
    Return { value: Option<Expr>, span: Span },
    Define { name: String, params: Vec<(String, Option<String>)>, return_type: Option<String>, docstring: Option<String>, body: Vec<Stmt>, span: Span },
    Init(Init),
    Class { name: String, init: Option<Init>, methods: Vec<Stmt>, docstring: Option<String>, span: Span },
    Import(Vec<(String, Span)>),
    Read { name: String, path: Expr },
    Write { content: Expr, path: Expr },
    Try { body: Vec<Stmt>, catch_var: String, catch_body: Vec<Stmt> },
    Export(Vec<String>),
    Expr(Expr),
    Pass,
}

#[derive(Debug, Clone)]
pub struct Init {
    pub params: Vec<(String, Option<String>)>,
    pub body: Vec<Stmt>,
    pub docstring: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Stmt>,
}

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    String(String),
    Bool(bool),
    Nothing,
    Variable { name: String, span: Span },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr>, span: Span },
    Unary { op: UnaryOp, operand: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Expr> },
    Index { object: Box<Expr>, index: Box<Expr> },
    Property { object: Box<Expr>, name: String },
    New { class: Box<Expr>, args: Vec<Expr> },
    Tell { object: Box<Expr>, method: String, args: Vec<Expr> },
    Qualified { name: String, module: String },
    List(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Ellipsis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Index { object: Box<Expr>, index: Box<Expr> },
    Property { object: Box<Expr>, name: String },
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let { name: String, value: Expr },
    Set { target: AssignTarget, value: Expr },
    Show(Expr),
    If { cond: Expr, then_branch: Vec<Stmt>, else_branch: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    For { var: String, iterable: Expr, body: Vec<Stmt> },
    Return(Option<Expr>),
    Define { name: String, params: Vec<(String, Option<String>)>, return_type: Option<String>, docstring: Option<String>, body: Vec<Stmt> },
    Init(Init),
    Class { name: String, init: Option<Init>, methods: Vec<Stmt>, docstring: Option<String> },
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

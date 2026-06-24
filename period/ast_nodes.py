"""Abstract syntax tree nodes for Period."""
from dataclasses import dataclass, field
from typing import List, Optional

from .errors import SourceSpan


@dataclass
class Node:
    span: SourceSpan


# Expressions -----------------------------------------------------------------

@dataclass
class Expr(Node):
    pass


@dataclass
class NumberLiteral(Expr):
    value: float


@dataclass
class StringLiteral(Expr):
    value: str


@dataclass
class BooleanLiteral(Expr):
    value: bool


@dataclass
class NothingLiteral(Expr):
    pass


@dataclass
class InputExpr(Expr):
    pass


@dataclass
class VariableExpr(Expr):
    name: str


@dataclass
class BinaryExpr(Expr):
    left: Expr
    operator: str
    right: Expr


@dataclass
class UnaryExpr(Expr):
    operator: str
    operand: Expr


@dataclass
class CallExpr(Expr):
    callee: Expr
    arguments: List[Expr]


@dataclass
class IndexExpr(Expr):
    object: Expr
    index: Expr


@dataclass
class ListExpr(Expr):
    elements: List[Expr]


@dataclass
class DictExpr(Expr):
    pairs: List[tuple]  # (Expr, Expr)


# Statements ------------------------------------------------------------------

@dataclass
class Stmt(Node):
    pass


@dataclass
class ExpressionStmt(Stmt):
    expression: Expr


@dataclass
class LetStmt(Stmt):
    name: str
    initializer: Expr


@dataclass
class SetStmt(Stmt):
    target: Expr
    value: Expr


@dataclass
class ShowStmt(Stmt):
    expression: Expr


@dataclass
class BlockStmt(Stmt):
    statements: List[Stmt]


@dataclass
class IfStmt(Stmt):
    condition: Expr
    then_branch: List[Stmt]
    else_branch: List[Stmt]


@dataclass
class WhileStmt(Stmt):
    condition: Expr
    body: List[Stmt]


@dataclass
class ReturnStmt(Stmt):
    value: Optional[Expr]


@dataclass
class DefineStmt(Stmt):
    name: str
    parameters: List[str]
    body: List[Stmt]


# Program ---------------------------------------------------------------------

@dataclass
class Program(Node):
    statements: List[Stmt] = field(default_factory=list)

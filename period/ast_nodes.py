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


@dataclass
class PropertyExpr(Expr):
    object: Expr
    name: str


@dataclass
class NewExpr(Expr):
    class_expr: Expr
    arguments: List[Expr]


@dataclass
class TellExpr(Expr):
    object: Expr
    method: str
    arguments: List[Expr]


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
    type_annotation: Optional[str] = None
    type_annotation_span: Optional[SourceSpan] = None
    is_default_initialization: bool = False
    name_span: Optional[SourceSpan] = None


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
class ImportStmt(Stmt):
    module_path: str
    module_span: SourceSpan


@dataclass
class DefineStmt(Stmt):
    name: str
    name_span: SourceSpan
    parameters: List[str]
    parameter_types: List[Optional[str]]
    parameter_type_spans: List[Optional[SourceSpan]]
    return_type: Optional[str]
    return_type_span: Optional[SourceSpan]
    body: List[Stmt]
    docstring: Optional[str] = None


@dataclass
class InitStmt(Stmt):
    parameters: List[str]
    parameter_types: List[Optional[str]]
    parameter_type_spans: List[Optional[SourceSpan]]
    body: List[Stmt]
    docstring: Optional[str] = None


@dataclass
class ClassStmt(Stmt):
    name: str
    name_span: SourceSpan
    body: List[Stmt]
    docstring: Optional[str] = None


# Program ---------------------------------------------------------------------

@dataclass
class Program(Node):
    statements: List[Stmt] = field(default_factory=list)

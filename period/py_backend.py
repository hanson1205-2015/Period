"""Python-code backend for a fast numeric subset of Period.

Translates supported Period programs into equivalent Python source and executes
them with CPython.  This is still *Period* running through its own backend; the
hot loop executes as Python bytecode rather than being interpreted one node at
a time.
"""
from __future__ import annotations

from typing import Any, List, Set, Tuple

from . import ast_nodes as ast


class PyBackendUnsupportedError(Exception):
    """Raised when the Python backend cannot handle an AST node."""

    def __init__(self, message: str, node: ast.Node):
        super().__init__(message)
        self.node = node


class _PyTranspiler:
    """Single-pass AST -> Python source transpiler."""

    def __init__(self):
        self.indent = 0
        self.lines: List[str] = []
        self._scope_stack: List[Set[str]] = []

    def _in_function(self) -> bool:
        return bool(self._scope_stack)

    def _locals(self) -> Set[str]:
        if not self._scope_stack:
            return set()
        return self._scope_stack[-1]

    def _add_local(self, name: str) -> None:
        if self._scope_stack:
            self._scope_stack[-1].add(name)

    def _is_local(self, name: str) -> bool:
        return name in self._locals()

    def _write(self, text: str) -> None:
        self.lines.append("    " * self.indent + text)

    def _expr(self, expr: ast.Expr) -> str:
        if isinstance(expr, ast.NumberLiteral):
            return repr(expr.value)
        if isinstance(expr, ast.BooleanLiteral):
            return "True" if expr.value else "False"
        if isinstance(expr, ast.NothingLiteral):
            return "None"
        if isinstance(expr, ast.VariableExpr):
            return expr.name
        if isinstance(expr, ast.UnaryExpr):
            operand = self._expr(expr.operand)
            if expr.operator == "-":
                return f"(-{operand})"
            if expr.operator == "not":
                return f"(not {operand})"
            raise PyBackendUnsupportedError(f"Unary '{expr.operator}'.", expr)
        if isinstance(expr, ast.BinaryExpr):
            return self._binary(expr)
        if isinstance(expr, ast.CallExpr):
            return self._call(expr)
        raise PyBackendUnsupportedError(
            f"Expression {type(expr).__name__} not supported.", expr
        )

    def _binary(self, expr: ast.BinaryExpr) -> str:
        op = expr.operator
        if op == "and":
            return f"({self._expr(expr.left)} and {self._expr(expr.right)})"
        if op == "or":
            return f"({self._expr(expr.left)} or {self._expr(expr.right)})"
        left = self._expr(expr.left)
        right = self._expr(expr.right)
        if op == "==":
            return f"({left} == {right})"
        if op == "!=":
            return f"({left} != {right})"
        if op == "<=":
            return f"({left} <= {right})"
        if op == ">=":
            return f"({left} >= {right})"
        if op == "**":
            return f"({left} ** {right})"
        return f"({left} {op} {right})"

    def _call(self, expr: ast.CallExpr) -> str:
        if isinstance(expr.callee, ast.VariableExpr) and expr.callee.name == "range":
            args = ", ".join(self._expr(a) for a in expr.arguments)
            return f"range({args})"
        if isinstance(expr.callee, ast.VariableExpr):
            callee = expr.callee.name
        else:
            callee = self._expr(expr.callee)
        args = ", ".join(self._expr(a) for a in expr.arguments)
        return f"{callee}({args})"

    def transpile(self, program: ast.Program) -> str:
        self._write("def __period_run():")
        self.indent += 1
        self._write("__period_output = []")
        top_globals = self._collect_assigned_names(program.statements)
        if top_globals:
            self._write("global " + ", ".join(sorted(top_globals)))
        for stmt in program.statements:
            self._stmt(stmt)
        self._write("return __period_output")
        self.indent -= 1
        self._write("")
        self._write("__period_out = __period_run()")
        return "\n".join(self.lines)

    def _stmt(self, stmt: ast.Stmt) -> None:
        if isinstance(stmt, ast.ExpressionStmt):
            self._write(self._expr(stmt.expression))
            return

        if isinstance(stmt, ast.LetStmt):
            init = self._expr(stmt.initializer)
            if stmt.type_annotation and stmt.is_default_initialization:
                init = self._default_for_type(stmt.type_annotation)
            self._add_local(stmt.name)
            self._assign(stmt.name, init)
            return

        if isinstance(stmt, ast.SetStmt):
            target = stmt.target
            if isinstance(target, ast.VariableExpr):
                self._assign(target.name, self._expr(stmt.value))
                return
            raise PyBackendUnsupportedError(
                f"Assignment target {type(target).__name__} not supported.", stmt
            )

        if isinstance(stmt, ast.ShowStmt):
            self._write(
                f"__period_output.append(__period_str({self._expr(stmt.expression)}))"
            )
            return

        if isinstance(stmt, ast.IfStmt):
            self._write(f"if {self._expr(stmt.condition)}:")
            self.indent += 1
            for s in stmt.then_branch:
                self._stmt(s)
            self.indent -= 1
            if stmt.else_branch:
                self._write("else:")
                self.indent += 1
                for s in stmt.else_branch:
                    self._stmt(s)
                self.indent -= 1
            return

        if isinstance(stmt, ast.WhileStmt):
            self._write(f"while {self._expr(stmt.condition)}:")
            self.indent += 1
            for s in stmt.body:
                self._stmt(s)
            self.indent -= 1
            return

        if isinstance(stmt, ast.ForStmt):
            if not self._is_range_call(stmt.iterable):
                raise PyBackendUnsupportedError(
                    "Only 'for ... in range with ...' is supported.", stmt
                )
            args = ", ".join(self._expr(a) for a in stmt.iterable.arguments)
            self._write(f"for {stmt.variable} in range({args}):")
            self.indent += 1
            for s in stmt.body:
                self._stmt(s)
            self.indent -= 1
            return

        if isinstance(stmt, ast.ReturnStmt):
            value = "None" if stmt.value is None else self._expr(stmt.value)
            self._write(f"return {value}")
            return

        if isinstance(stmt, ast.DefineStmt):
            self._compile_function(stmt)
            return

        if isinstance(stmt, ast.BlockStmt):
            for s in stmt.statements:
                self._stmt(s)
            return

        raise PyBackendUnsupportedError(
            f"Statement {type(stmt).__name__} not supported.", stmt
        )

    def _compile_function(self, stmt: ast.DefineStmt) -> None:
        params = ", ".join(stmt.parameters)
        self._write(f"def {stmt.name}({params}):")
        self.indent += 1

        # Determine function-local names (parameters + let-declared variables).
        locals_: Set[str] = set(stmt.parameters)
        locals_.update(self._collect_let_names(stmt.body))
        self._scope_stack.append(locals_)

        # Any variable assigned here that is not local must be a global.
        globals_assigned = self._collect_assigned_names(stmt.body) - locals_
        if globals_assigned:
            self._write("global " + ", ".join(sorted(globals_assigned)))

        for body_stmt in stmt.body:
            self._stmt(body_stmt)
        self.indent -= 1
        self._scope_stack.pop()

    def _assign(self, name: str, value: str) -> None:
        # Globals are pre-declared at the start of each function/top-level block,
        # so assignments here only need to be actual stores.
        self._write(f"{name} = {value}")

    @staticmethod
    def _collect_let_names(stmts: List[ast.Stmt]) -> Set[str]:
        names: Set[str] = set()
        for stmt in stmts:
            if isinstance(stmt, ast.LetStmt):
                names.add(stmt.name)
            elif isinstance(stmt, (ast.IfStmt, ast.WhileStmt, ast.ForStmt, ast.BlockStmt)):
                names.update(_PyTranspiler._collect_let_names(getattr(stmt, "then_branch", [])))
                names.update(_PyTranspiler._collect_let_names(getattr(stmt, "else_branch", [])))
                names.update(_PyTranspiler._collect_let_names(getattr(stmt, "body", [])))
        return names

    @staticmethod
    def _collect_assigned_names(stmts: List[ast.Stmt]) -> Set[str]:
        names: Set[str] = set()
        for stmt in stmts:
            if isinstance(stmt, ast.SetStmt) and isinstance(stmt.target, ast.VariableExpr):
                names.add(stmt.target.name)
            elif isinstance(stmt, ast.LetStmt):
                names.add(stmt.name)
            elif isinstance(stmt, (ast.IfStmt, ast.WhileStmt, ast.ForStmt, ast.BlockStmt)):
                names.update(_PyTranspiler._collect_assigned_names(getattr(stmt, "then_branch", [])))
                names.update(_PyTranspiler._collect_assigned_names(getattr(stmt, "else_branch", [])))
                names.update(_PyTranspiler._collect_assigned_names(getattr(stmt, "body", [])))
        return names

    @staticmethod
    def _default_for_type(type_name: str) -> str:
        defaults = {
            "string": '""',
            "number": "0",
            "integer": "0",
            "boolean": "False",
            "list": "[]",
            "dictionary": "{}",
        }
        return defaults.get(type_name, "None")

    @staticmethod
    def _is_range_call(expr: ast.Expr) -> bool:
        return (
            isinstance(expr, ast.CallExpr)
            and isinstance(expr.callee, ast.VariableExpr)
            and expr.callee.name == "range"
        )


def transpile(program: ast.Program) -> str:
    """Return Python source equivalent to *program* (numeric subset)."""
    return _PyTranspiler().transpile(program)


def __period_str(value: Any) -> str:
    if value is None:
        return "nothing"
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def run(program: ast.Program) -> Tuple[bool, List[str], str]:
    """Run *program* via the Python backend.

    Returns ``(success, output_lines, error_message)``.
    """
    try:
        source = transpile(program)
    except PyBackendUnsupportedError as exc:
        return False, [], str(exc)

    namespace: dict = {"__period_str": __period_str}
    try:
        exec(source, namespace)
    except Exception as exc:
        return False, [], f"{type(exc).__name__}: {exc}"

    result = namespace.get("__period_out", [])
    lines = [__period_str(value) for value in result]
    return True, lines, ""

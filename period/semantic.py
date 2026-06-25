"""Semantic analysis for Period, including identifier kind resolution."""
from typing import Dict, List, Optional, Set, Tuple

from . import ast_nodes as ast
from .errors import Diagnostic, SourceSpan

BUILTINS: Set[str] = {
    "length",
    "string",
    "number",
    "type",
    "input",
    "show",
}

# Recognized Period type names. They are treated as predefined identifiers and
# highlighted as classes. They do not have to be runtime built-ins.
TYPE_NAMES: Set[str] = {
    "any",
    "never",
    "nothing",
    "boolean",
    "integer",
    "number",
    "string",
    "list",
    "dictionary",
    "function",
    "class",
}

# Token kinds used for LSP semantic highlighting.
TOKEN_FUNCTION = "function"
TOKEN_CLASS = "class"
TOKEN_VARIABLE = "variable"
TOKEN_PARAMETER = "parameter"
TOKEN_PROPERTY = "property"
TOKEN_METHOD = "method"
TOKEN_BUILTIN = "builtin"


class SemanticChecker:
    """Walks the AST and reports undefined variable usages."""

    def __init__(self):
        self.diagnostics: List[Diagnostic] = []
        self.scopes: List[Dict[str, str]] = []
        self.tokens: List[Tuple[SourceSpan, str, bool]] = []
        self.filename: str = "<stdin>"

    def check(self, program: ast.Program, filename: str = "<stdin>") -> List[Diagnostic]:
        self.diagnostics = []
        self.tokens = []
        self.scopes = [self._builtin_scope()]
        self.filename = filename

        # First pass: register all top-level function and class names so they can
        # be referenced before their definition.
        for stmt in program.statements:
            if isinstance(stmt, ast.DefineStmt):
                self.scopes[0][stmt.name] = TOKEN_FUNCTION
            elif isinstance(stmt, ast.ClassStmt):
                self.scopes[0][stmt.name] = TOKEN_CLASS

        for stmt in program.statements:
            self._visit_stmt(stmt)

        return self.diagnostics

    def semantic_tokens(self, program: ast.Program, filename: str = "<stdin>") -> List[Tuple[SourceSpan, str, bool]]:
        """Return a list of (span, kind, is_declaration) tuples for semantic highlighting."""
        self.check(program, filename)
        return self.tokens

    def _builtin_scope(self) -> Dict[str, str]:
        scope = {name: TOKEN_BUILTIN for name in BUILTINS}
        for name in TYPE_NAMES:
            scope[name] = TOKEN_CLASS
        return scope

    def _declare(self, name: str, kind: str):
        if self.scopes:
            self.scopes[-1][name] = kind

    def _lookup(self, name: str) -> Optional[str]:
        for scope in reversed(self.scopes):
            if name in scope:
                return scope[name]
        return None

    def _is_defined(self, name: str) -> bool:
        return self._lookup(name) is not None

    def _error(self, name: str, span: SourceSpan):
        self.diagnostics.append(
            Diagnostic(
                f"Undefined variable '{name}'.",
                span,
                "error",
            )
        )

    def _infer_expr_type(self, expr: ast.Expr) -> Optional[str]:
        """Statically infer the Period type name of an expression, if obvious."""
        if isinstance(expr, ast.NumberLiteral):
            return "number"
        if isinstance(expr, ast.StringLiteral):
            return "string"
        if isinstance(expr, ast.BooleanLiteral):
            return "boolean"
        if isinstance(expr, ast.NothingLiteral):
            return "nothing"
        if isinstance(expr, ast.ListExpr):
            return "list"
        if isinstance(expr, ast.DictExpr):
            return "dictionary"
        if isinstance(expr, ast.NewExpr):
            if isinstance(expr.class_expr, ast.VariableExpr):
                return f"instance of {expr.class_expr.name}"
            return "instance"
        if isinstance(expr, ast.VariableExpr):
            if expr.name in TYPE_NAMES:
                return expr.name
        return None

    def _check_let_type(self, stmt: ast.LetStmt):
        """Report a diagnostic if the initializer doesn't match the declared type."""
        expected = stmt.type_annotation
        if expected == "any":
            return
        actual = self._infer_expr_type(stmt.initializer)
        if actual is None:
            return
        if expected == actual:
            return
        # 'number' accepts integer values.
        if expected == "number" and actual == "integer":
            return
        # Class annotations match instances of that class.
        if actual == f"instance of {expected}":
            return
        self.diagnostics.append(
            Diagnostic(
                f"Type mismatch: expected '{expected}' but got '{actual}'.",
                stmt.initializer.span,
                "error",
            )
        )

    def _add_token(self, span: SourceSpan, kind: str, is_declaration: bool = False):
        self.tokens.append((span, kind, is_declaration))

    def _visit_stmt(self, stmt: ast.Stmt):
        if isinstance(stmt, ast.ExpressionStmt):
            self._visit_expr(stmt.expression)
        elif isinstance(stmt, ast.LetStmt):
            self._visit_expr(stmt.initializer)
            if stmt.type_annotation_span is not None:
                self._add_token(stmt.type_annotation_span, TOKEN_CLASS)
            if stmt.type_annotation is not None:
                self._check_let_type(stmt)
            self._declare(stmt.name, TOKEN_VARIABLE)
        elif isinstance(stmt, ast.SetStmt):
            self._visit_expr(stmt.value)
            self._visit_set_target(stmt.target)
        elif isinstance(stmt, ast.ShowStmt):
            self._visit_expr(stmt.expression)
        elif isinstance(stmt, ast.BlockStmt):
            self._scope(self._visit_stmts, stmt.statements)
        elif isinstance(stmt, ast.IfStmt):
            self._visit_expr(stmt.condition)
            self._scope(self._visit_stmts, stmt.then_branch)
            self._scope(self._visit_stmts, stmt.else_branch)
        elif isinstance(stmt, ast.WhileStmt):
            self._visit_expr(stmt.condition)
            self._scope(self._visit_stmts, stmt.body)
        elif isinstance(stmt, ast.ReturnStmt):
            if stmt.value is not None:
                self._visit_expr(stmt.value)
        elif isinstance(stmt, ast.DefineStmt):
            self._add_token(stmt.name_span, TOKEN_FUNCTION, is_declaration=True)
            self._declare(stmt.name, TOKEN_FUNCTION)
            self._scope(self._visit_function_body, stmt)
        elif isinstance(stmt, ast.ClassStmt):
            self._add_token(stmt.name_span, TOKEN_CLASS, is_declaration=True)
            self._declare(stmt.name, TOKEN_CLASS)
            self._visit_class_body(stmt)
        elif isinstance(stmt, ast.ImportStmt):
            self._visit_import(stmt)
        elif isinstance(stmt, ast.InitStmt):
            # Init outside a class body is a parse/runtime concern; nothing to check here.
            pass

    def _visit_import(self, stmt: ast.ImportStmt):
        from .module_loader import resolve_module

        resolved = resolve_module(stmt.module_path, self.filename)
        if resolved is None:
            self.diagnostics.append(
                Diagnostic(
                    f"Module '{stmt.module_path}' not found.",
                    stmt.span,
                    "error",
                )
            )
            return

        if isinstance(resolved, str):
            self._declare_builtin_module_exports(resolved)
        else:
            self._declare_file_module_exports(resolved)

    def _declare_builtin_module_exports(self, name: str):
        import importlib

        try:
            mod = importlib.import_module(f"period.stdlib.{name}")
        except Exception as exc:
            self.diagnostics.append(
                Diagnostic(
                    f"Could not load built-in module '{name}': {exc}.",
                    SourceSpan(1, 1, 1),
                    "error",
                )
            )
            return

        exports = getattr(mod, "EXPORTS", [])
        for export in exports:
            if not hasattr(mod, export):
                continue
            value = getattr(mod, export)
            kind = TOKEN_FUNCTION if callable(value) else TOKEN_VARIABLE
            self._declare(export, kind)

    def _declare_file_module_exports(self, path):
        from .lexer import Lexer
        from .parser import Parser

        source = path.read_text(encoding="utf-8")
        lexer = Lexer(source, str(path))
        tokens = lexer.scan()
        diagnostics = list(lexer.diagnostics)

        parser = Parser(tokens, source, str(path))
        program = parser.parse()
        diagnostics.extend(parser.diagnostics)

        if diagnostics:
            for diag in diagnostics:
                self.diagnostics.append(diag)
            return

        for s in program.statements:
            if isinstance(s, ast.DefineStmt):
                self._declare(s.name, TOKEN_FUNCTION)
            elif isinstance(s, ast.ClassStmt):
                self._declare(s.name, TOKEN_CLASS)
            elif isinstance(s, ast.LetStmt):
                self._declare(s.name, TOKEN_VARIABLE)

    def _visit_stmts(self, statements: List[ast.Stmt]):
        for stmt in statements:
            self._visit_stmt(stmt)

    def _visit_function_body(self, stmt: ast.DefineStmt):
        for param, param_type in zip(stmt.parameters, stmt.parameter_types):
            self._declare(param, TOKEN_PARAMETER)
        for param_type_span in stmt.parameter_type_spans:
            if param_type_span is not None:
                self._add_token(param_type_span, TOKEN_CLASS)
        if stmt.return_type_span is not None:
            self._add_token(stmt.return_type_span, TOKEN_CLASS)
        self._visit_stmts(stmt.body)

    def _visit_class_body(self, stmt: ast.ClassStmt):
        for member in stmt.body:
            if isinstance(member, ast.InitStmt):
                self._scope(self._visit_init_body, member)
            elif isinstance(member, ast.DefineStmt):
                self._add_token(member.name_span, TOKEN_METHOD, is_declaration=True)
                self._scope(self._visit_method_body, member)

    def _visit_init_body(self, stmt: ast.InitStmt):
        self._declare("this", TOKEN_VARIABLE)
        for param, param_type in zip(stmt.parameters, stmt.parameter_types):
            self._declare(param, TOKEN_PARAMETER)
        for param_type_span in stmt.parameter_type_spans:
            if param_type_span is not None:
                self._add_token(param_type_span, TOKEN_CLASS)
        self._visit_stmts(stmt.body)

    def _visit_method_body(self, stmt: ast.DefineStmt):
        self._declare("this", TOKEN_VARIABLE)
        for param, param_type in zip(stmt.parameters, stmt.parameter_types):
            self._declare(param, TOKEN_PARAMETER)
        for param_type_span in stmt.parameter_type_spans:
            if param_type_span is not None:
                self._add_token(param_type_span, TOKEN_CLASS)
        if stmt.return_type_span is not None:
            self._add_token(stmt.return_type_span, TOKEN_CLASS)
        self._visit_stmts(stmt.body)

    def _scope(self, fn, *args):
        self.scopes.append({})
        try:
            fn(*args)
        finally:
            self.scopes.pop()

    def _visit_set_target(self, target: ast.Expr):
        if isinstance(target, ast.VariableExpr):
            if not self._is_defined(target.name):
                self._error(target.name, target.span)
        elif isinstance(target, ast.PropertyExpr):
            self._visit_expr(target.object)
            self._add_token(target.span, TOKEN_PROPERTY)
        else:
            self._visit_expr(target)

    def _visit_expr(self, expr: ast.Expr):
        if isinstance(expr, ast.VariableExpr):
            if not self._is_defined(expr.name):
                self._error(expr.name, expr.span)
            kind = self._lookup(expr.name) or TOKEN_VARIABLE
            self._add_token(expr.span, kind)
        elif isinstance(expr, ast.BinaryExpr):
            self._visit_expr(expr.left)
            self._visit_expr(expr.right)
        elif isinstance(expr, ast.UnaryExpr):
            self._visit_expr(expr.operand)
        elif isinstance(expr, ast.CallExpr):
            self._visit_expr(expr.callee)
            for arg in expr.arguments:
                self._visit_expr(arg)
        elif isinstance(expr, ast.IndexExpr):
            self._visit_expr(expr.object)
            self._visit_expr(expr.index)
        elif isinstance(expr, ast.ListExpr):
            for el in expr.elements:
                self._visit_expr(el)
        elif isinstance(expr, ast.DictExpr):
            for key, value in expr.pairs:
                self._visit_expr(key)
                self._visit_expr(value)
        elif isinstance(expr, ast.PropertyExpr):
            self._visit_expr(expr.object)
            if isinstance(expr.object, ast.VariableExpr):
                kind = self._lookup(expr.object.name)
                if kind in (TOKEN_BUILTIN, TOKEN_FUNCTION, TOKEN_CLASS):
                    self.diagnostics.append(
                        Diagnostic(
                            f"Cannot access property on {kind}.",
                            expr.span,
                            "error",
                        )
                    )
            self._add_token(expr.span, TOKEN_PROPERTY)
        elif isinstance(expr, ast.NewExpr):
            self._visit_expr(expr.class_expr)
            for arg in expr.arguments:
                self._visit_expr(arg)
        elif isinstance(expr, ast.TellExpr):
            self._visit_expr(expr.object)
            self._add_token(expr.span, TOKEN_METHOD)
            for arg in expr.arguments:
                self._visit_expr(arg)

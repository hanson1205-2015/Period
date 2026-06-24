"""Tree-walking interpreter for the Period programming language."""
import math
from typing import Any, Dict, List, Optional

from . import ast_nodes as ast
from .errors import RuntimeError, SourceSpan


class ReturnValue(Exception):
    """Control-flow exception used to return values from functions."""

    def __init__(self, value: Any):
        self.value = value


class PeriodFunction:
    """User-defined function value."""

    def __init__(self, declaration: ast.DefineStmt, closure: "Environment"):
        self.declaration = declaration
        self.closure = closure

    def __repr__(self) -> str:
        return f"<function {self.declaration.name}>"


class PeriodBuiltIn:
    """Built-in callable value."""

    def __init__(self, name: str, arity: int, fn):
        self.name = name
        self.arity = arity
        self.fn = fn

    def __repr__(self) -> str:
        return f"<built-in {self.name}>"


class Environment:
    """Variable environment with support for nested scopes."""

    def __init__(self, enclosing: Optional["Environment"] = None):
        self.values: Dict[str, Any] = {}
        self.enclosing = enclosing

    def define(self, name: str, value: Any):
        self.values[name] = value

    def get(self, name: str, span: SourceSpan) -> Any:
        if name in self.values:
            return self.values[name]
        if self.enclosing is not None:
            return self.enclosing.get(name, span)
        raise RuntimeError(f"Undefined variable '{name}'.", span)

    def set(self, name: str, value: Any, span: SourceSpan):
        if name in self.values:
            self.values[name] = value
            return
        if self.enclosing is not None:
            self.enclosing.set(name, value, span)
            return
        raise RuntimeError(f"Undefined variable '{name}'.", span)


class Interpreter:
    """Execute a Period AST."""

    def __init__(self):
        self.globals = Environment()
        self.environment = self.globals
        self.output: List[str] = []
        self._install_builtins()

    def _install_builtins(self):
        self.globals.define(
            "length",
            PeriodBuiltIn(
                "length",
                1,
                lambda args, span: self._builtin_length(args[0], span),
            ),
        )
        self.globals.define(
            "string",
            PeriodBuiltIn(
                "string",
                1,
                lambda args, span: self._to_string(args[0], span),
            ),
        )
        self.globals.define(
            "number",
            PeriodBuiltIn(
                "number",
                1,
                lambda args, span: self._to_number(args[0], span),
            ),
        )
        self.globals.define(
            "type",
            PeriodBuiltIn(
                "type",
                1,
                lambda args, span: self._type_name(args[0], span),
            ),
        )
        self.globals.define(
            "input",
            PeriodBuiltIn(
                "input",
                0,
                lambda args, span: self._read_input(span),
            ),
        )

    # Built-in implementations ------------------------------------------------

    def _builtin_length(self, value: Any, span: SourceSpan) -> int:
        if isinstance(value, (str, list, dict)):
            return len(value)
        raise RuntimeError(f"Cannot get length of {self._type_name(value, span)}.", span)

    def _to_string(self, value: Any, span: SourceSpan) -> str:
        if value is None:
            return "nothing"
        if isinstance(value, bool):
            return "true" if value else "false"
        if isinstance(value, (list, dict)):
            return self._pretty(value)
        return str(value)

    def _to_number(self, value: Any, span: SourceSpan) -> float:
        if isinstance(value, bool):
            return 1.0 if value else 0.0
        if isinstance(value, (int, float)):
            return value
        if isinstance(value, str):
            try:
                if "." in value:
                    return float(value)
                return int(value)
            except ValueError:
                raise RuntimeError(f"Cannot convert '{value}' to a number.", span)
        raise RuntimeError(f"Cannot convert {self._type_name(value, span)} to a number.", span)

    def _type_name(self, value: Any, span: SourceSpan) -> str:
        if value is None:
            return "nothing"
        if isinstance(value, bool):
            return "boolean"
        if isinstance(value, int):
            return "integer"
        if isinstance(value, float):
            return "number"
        if isinstance(value, str):
            return "string"
        if isinstance(value, list):
            return "list"
        if isinstance(value, dict):
            return "dictionary"
        if isinstance(value, PeriodFunction):
            return "function"
        if isinstance(value, PeriodBuiltIn):
            return "built-in"
        return "unknown"

    def _read_input(self, span: SourceSpan) -> str:
        try:
            return input()
        except EOFError:
            return ""

    def _pretty(self, value: Any) -> str:
        if value is None:
            return "nothing"
        if isinstance(value, bool):
            return "true" if value else "false"
        if isinstance(value, list):
            return "[" + ", ".join(self._pretty(v) for v in value) + "]"
        if isinstance(value, dict):
            items = ", ".join(f"{self._pretty(k)}: {self._pretty(v)}" for k, v in value.items())
            return "{" + items + "}"
        return str(value)

    # Public API --------------------------------------------------------------

    def interpret(self, program: ast.Program) -> List[str]:
        for stmt in program.statements:
            self._execute(stmt)
        return self.output

    def execute_block(self, statements: List[ast.Stmt], environment: Environment):
        previous = self.environment
        try:
            self.environment = environment
            for stmt in statements:
                self._execute(stmt)
        finally:
            self.environment = previous

    # Statement execution -----------------------------------------------------

    def _execute(self, stmt: ast.Stmt):
        if isinstance(stmt, ast.ExpressionStmt):
            self._evaluate(stmt.expression)
            return
        if isinstance(stmt, ast.LetStmt):
            value = self._evaluate(stmt.initializer)
            self.environment.define(stmt.name, value)
            return
        if isinstance(stmt, ast.SetStmt):
            value = self._evaluate(stmt.value)
            self._assign_target(stmt.target, value, stmt.span)
            return
        if isinstance(stmt, ast.ShowStmt):
            value = self._evaluate(stmt.expression)
            text = self._to_string(value, stmt.expression.span)
            self.output.append(text)
            print(text)
            return
        if isinstance(stmt, ast.IfStmt):
            condition = self._evaluate(stmt.condition)
            if self._is_truthy(condition):
                self.execute_block(stmt.then_branch, Environment(self.environment))
            elif stmt.else_branch:
                self.execute_block(stmt.else_branch, Environment(self.environment))
            return
        if isinstance(stmt, ast.WhileStmt):
            while self._is_truthy(self._evaluate(stmt.condition)):
                self.execute_block(stmt.body, Environment(self.environment))
            return
        if isinstance(stmt, ast.ReturnStmt):
            value = None
            if stmt.value is not None:
                value = self._evaluate(stmt.value)
            raise ReturnValue(value)
        if isinstance(stmt, ast.DefineStmt):
            function = PeriodFunction(stmt, self.environment)
            self.environment.define(stmt.name, function)
            return
        raise RuntimeError(f"Unknown statement type: {type(stmt).__name__}.", stmt.span)

    def _assign_target(self, target: ast.Expr, value: Any, span: SourceSpan):
        if isinstance(target, ast.VariableExpr):
            self.environment.set(target.name, value, span)
            return
        if isinstance(target, ast.IndexExpr):
            obj = self._evaluate(target.object)
            index = self._evaluate(target.index)
            if isinstance(obj, list):
                if not isinstance(index, int) or isinstance(index, bool):
                    raise RuntimeError("List index must be an integer.", target.index.span)
                try:
                    obj[index] = value
                except IndexError:
                    raise RuntimeError(f"List index {index} out of range.", target.index.span)
                return
            if isinstance(obj, dict):
                obj[index] = value
                return
            raise RuntimeError(f"Cannot index into {self._type_name(obj, span)}.", target.object.span)
        raise RuntimeError("Invalid assignment target.", span)

    # Expression evaluation ---------------------------------------------------

    def _evaluate(self, expr: ast.Expr) -> Any:
        if isinstance(expr, ast.NumberLiteral):
            return expr.value
        if isinstance(expr, ast.StringLiteral):
            return expr.value
        if isinstance(expr, ast.BooleanLiteral):
            return expr.value
        if isinstance(expr, ast.NothingLiteral):
            return None
        if isinstance(expr, ast.InputExpr):
            return self._read_input(expr.span)
        if isinstance(expr, ast.VariableExpr):
            return self.environment.get(expr.name, expr.span)
        if isinstance(expr, ast.ListExpr):
            return [self._evaluate(e) for e in expr.elements]
        if isinstance(expr, ast.DictExpr):
            return {self._evaluate(k): self._evaluate(v) for k, v in expr.pairs}
        if isinstance(expr, ast.UnaryExpr):
            return self._eval_unary(expr)
        if isinstance(expr, ast.BinaryExpr):
            return self._eval_binary(expr)
        if isinstance(expr, ast.CallExpr):
            return self._eval_call(expr)
        if isinstance(expr, ast.IndexExpr):
            return self._eval_index(expr)
        raise RuntimeError(f"Unknown expression type: {type(expr).__name__}.", expr.span)

    def _eval_unary(self, expr: ast.UnaryExpr) -> Any:
        operand = self._evaluate(expr.operand)
        if expr.operator == "-":
            if isinstance(operand, bool):
                return -1.0 if operand else 0.0
            if isinstance(operand, (int, float)):
                return -operand
            raise RuntimeError(f"Cannot negate {self._type_name(operand, expr.operand.span)}.", expr.operand.span)
        if expr.operator == "not":
            return not self._is_truthy(operand)
        raise RuntimeError(f"Unknown unary operator '{expr.operator}'.", expr.span)

    def _eval_binary(self, expr: ast.BinaryExpr) -> Any:
        op = expr.operator

        # Short-circuit logical operators.
        if op == "and":
            left = self._evaluate(expr.left)
            if not self._is_truthy(left):
                return left
            return self._evaluate(expr.right)
        if op == "or":
            left = self._evaluate(expr.left)
            if self._is_truthy(left):
                return left
            return self._evaluate(expr.right)

        left = self._evaluate(expr.left)
        right = self._evaluate(expr.right)

        if op == "+":
            if isinstance(left, str) and isinstance(right, str):
                return left + right
            if isinstance(left, list) and isinstance(right, list):
                return left + right
            return self._numeric_op(left, right, lambda a, b: a + b, expr.span)
        if op == "-":
            return self._numeric_op(left, right, lambda a, b: a - b, expr.span)
        if op == "*":
            if isinstance(left, str) and isinstance(right, int) and not isinstance(right, bool):
                return left * right
            if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, str):
                return left * right
            return self._numeric_op(left, right, lambda a, b: a * b, expr.span)
        if op == "/":
            return self._numeric_op(left, right, lambda a, b: self._safe_divide(a, b, expr.span), expr.span)
        if op == "%":
            return self._numeric_op(left, right, lambda a, b: self._safe_modulo(a, b, expr.span), expr.span)
        if op == "**":
            return self._numeric_op(left, right, lambda a, b: a ** b, expr.span)

        if op == "==":
            return self._is_equal(left, right)
        if op == "!=":
            return not self._is_equal(left, right)

        if op in {"<", ">", "<=", ">="}:
            self._check_numbers(left, right, expr.span)
            if op == "<":
                return left < right
            if op == ">":
                return left > right
            if op == "<=":
                return left <= right
            return left >= right

        raise RuntimeError(f"Unknown binary operator '{op}'.", expr.span)

    def _eval_call(self, expr: ast.CallExpr) -> Any:
        callee = self._evaluate(expr.callee)
        arguments = [self._evaluate(arg) for arg in expr.arguments]

        if isinstance(callee, PeriodBuiltIn):
            if callee.arity != len(arguments):
                raise RuntimeError(
                    f"Built-in '{callee.name}' expects {callee.arity} argument(s) but got {len(arguments)}.",
                    expr.span,
                )
            return callee.fn(arguments, expr.span)

        if isinstance(callee, PeriodFunction):
            decl = callee.declaration
            if len(decl.parameters) != len(arguments):
                raise RuntimeError(
                    f"Function '{decl.name}' expects {len(decl.parameters)} argument(s) but got {len(arguments)}.",
                    expr.span,
                )
            env = Environment(callee.closure)
            for param, arg in zip(decl.parameters, arguments):
                env.define(param, arg)
            try:
                self.execute_block(decl.body, env)
            except ReturnValue as ret:
                return ret.value
            return None

        raise RuntimeError(f"Cannot call {self._type_name(callee, expr.span)}.", expr.span)

    def _eval_index(self, expr: ast.IndexExpr) -> Any:
        obj = self._evaluate(expr.object)
        index = self._evaluate(expr.index)
        if isinstance(obj, str):
            if not isinstance(index, int) or isinstance(index, bool):
                raise RuntimeError("String index must be an integer.", expr.index.span)
            try:
                return obj[index]
            except IndexError:
                raise RuntimeError(f"String index {index} out of range.", expr.index.span)
        if isinstance(obj, list):
            if not isinstance(index, int) or isinstance(index, bool):
                raise RuntimeError("List index must be an integer.", expr.index.span)
            try:
                return obj[index]
            except IndexError:
                raise RuntimeError(f"List index {index} out of range.", expr.index.span)
        if isinstance(obj, dict):
            try:
                return obj[index]
            except KeyError:
                raise RuntimeError(f"Key {self._pretty(index)} not found in dictionary.", expr.index.span)
        raise RuntimeError(f"Cannot index into {self._type_name(obj, expr.span)}.", expr.object.span)

    # Numeric helpers ---------------------------------------------------------

    def _numeric_op(self, left: Any, right: Any, op, span: SourceSpan):
        self._check_numbers(left, right, span)
        return op(left, right)

    def _check_numbers(self, left: Any, right: Any, span: SourceSpan):
        if isinstance(left, bool) or isinstance(right, bool):
            raise RuntimeError("Boolean values cannot be used in arithmetic operations.", span)
        if not isinstance(left, (int, float)) or not isinstance(right, (int, float)):
            raise RuntimeError("Operands must be numbers.", span)

    def _safe_divide(self, a: float, b: float, span: SourceSpan) -> float:
        if b == 0:
            raise RuntimeError("Division by zero.", span)
        return a / b

    def _safe_modulo(self, a: float, b: float, span: SourceSpan) -> float:
        if b == 0:
            raise RuntimeError("Modulo by zero.", span)
        return a % b

    # Utilities ---------------------------------------------------------------

    def _is_truthy(self, value: Any) -> bool:
        if value is None:
            return False
        if isinstance(value, bool):
            return value
        if isinstance(value, (int, float)):
            return value != 0
        if isinstance(value, str):
            return len(value) > 0
        if isinstance(value, (list, dict)):
            return len(value) > 0
        return True

    def _is_equal(self, a: Any, b: Any) -> bool:
        if a is None and b is None:
            return True
        if a is None or b is None:
            return False
        if isinstance(a, bool) or isinstance(b, bool):
            return bool(a) == bool(b)
        return a == b

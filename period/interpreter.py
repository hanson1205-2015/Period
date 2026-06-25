"""Tree-walking interpreter for the Period programming language."""
import importlib
import inspect
import math
from pathlib import Path
from typing import Any, Dict, List, Optional

from . import ast_nodes as ast
from .errors import RuntimeError, SourceSpan
from .semantic import BUILTINS, TYPE_NAMES


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


class PeriodType:
    """A named type value, e.g. boolean, any, never."""

    def __init__(self, name: str):
        self.name = name

    def __repr__(self) -> str:
        return f"<type {self.name}>"


class PeriodClass:
    """User-defined class value."""

    def __init__(self, name: str, init: Optional[ast.InitStmt], methods: Dict[str, ast.DefineStmt]):
        self.name = name
        self.init = init
        self.methods = methods

    def __repr__(self) -> str:
        return f"<class {self.name}>"


class PeriodInstance:
    """An instance of a user-defined class."""

    def __init__(self, klass: PeriodClass):
        self.klass = klass
        self.fields: Dict[str, Any] = {}

    def __repr__(self) -> str:
        return f"<instance of {self.klass.name}>"


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
        self.filename: str = "<stdin>"
        self.modules: Dict[str, Environment] = {}
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
        # Non-callable type names are also available as values (e.g. boolean, any, never).
        for type_name in TYPE_NAMES:
            if type_name not in BUILTINS:
                self.globals.define(type_name, PeriodType(type_name))

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
        if isinstance(value, PeriodInstance):
            return self._pretty_instance(value)
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
        if isinstance(value, PeriodClass):
            return "class"
        if isinstance(value, PeriodInstance):
            return f"instance of {value.klass.name}"
        return "unknown"

    def _check_type(self, value: Any, expected: str, span: SourceSpan):
        """Raise a runtime error if value does not match the expected type annotation."""
        if expected is None:
            return
        if expected == "any":
            return
        actual = self._type_name(value, span)
        if expected == "never":
            raise RuntimeError(
                f"Type mismatch: expected 'never' but got '{actual}'.",
                span,
            )
        if expected == actual:
            return
        # 'number' accepts both integer and number.
        if expected == "number" and actual == "integer":
            return
        # 'function' accepts both user functions and built-ins.
        if expected == "function" and actual in ("function", "built-in"):
            return
        # Class annotations match instances of that class.
        if actual == f"instance of {expected}":
            return
        raise RuntimeError(
            f"Type mismatch: expected '{expected}' but got '{actual}'.",
            span,
        )

    def _default_for_type(self, type_name: str) -> Any:
        """Return a sensible default value for a typed declaration without an initializer."""
        if type_name == "string":
            return ""
        if type_name == "number":
            return 0
        if type_name == "integer":
            return 0
        if type_name == "boolean":
            return False
        if type_name == "list":
            return []
        if type_name == "dictionary":
            return {}
        return None

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
        if isinstance(value, PeriodInstance):
            return self._pretty_instance(value)
        return str(value)

    def _pretty_instance(self, instance: PeriodInstance) -> str:
        items = ", ".join(f"{k}: {self._pretty(v)}" for k, v in instance.fields.items())
        return f"<{instance.klass.name} {items}>"

    # Public API --------------------------------------------------------------

    def interpret(self, program: ast.Program, filename: Optional[str] = None) -> List[str]:
        if filename is not None:
            self.filename = filename
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
            if stmt.type_annotation is not None:
                if value is None and stmt.is_default_initialization:
                    value = self._default_for_type(stmt.type_annotation)
                self._check_type(value, stmt.type_annotation, stmt.span)
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
        if isinstance(stmt, ast.ClassStmt):
            init: Optional[ast.InitStmt] = None
            methods: Dict[str, ast.DefineStmt] = {}
            for member in stmt.body:
                if isinstance(member, ast.InitStmt):
                    init = member
                elif isinstance(member, ast.DefineStmt):
                    methods[member.name] = member
                else:
                    raise RuntimeError("Class body may only contain 'init' and method definitions.", member.span)
            klass = PeriodClass(stmt.name, init, methods)
            self.environment.define(stmt.name, klass)
            return
        if isinstance(stmt, ast.ImportStmt):
            self._import_module(stmt.module_path)
            return
        if isinstance(stmt, ast.InitStmt):
            raise RuntimeError("'init' may only appear inside a class body.", stmt.span)
        raise RuntimeError(f"Unknown statement type: {type(stmt).__name__}.", stmt.span)

    def _import_module(self, module_path: str):
        from .module_loader import resolve_module

        resolved = resolve_module(module_path, self.filename)
        if resolved is None:
            raise RuntimeError(f"Module '{module_path}' not found.", SourceSpan(0, 0, 0))

        if isinstance(resolved, Path):
            cache_key = str(resolved.resolve())
            if cache_key in self.modules:
                module_env = self.modules[cache_key]
            else:
                module_env = self._load_file_module(resolved)
                self.modules[cache_key] = module_env
        else:
            if resolved in self.modules:
                module_env = self.modules[resolved]
            else:
                module_env = self._load_builtin_module(resolved)
                self.modules[resolved] = module_env

        for name, value in module_env.values.items():
            self.environment.define(name, value)

    def _load_builtin_module(self, name: str) -> Environment:
        from .lexer import Lexer
        from .parser import Parser
        from .semantic import SemanticChecker

        try:
            mod = importlib.import_module(f"period.stdlib.{name}")
        except Exception as exc:
            raise RuntimeError(f"Could not load built-in module '{name}': {exc}.", SourceSpan(0, 0, 0))

        env = Environment()
        exports = getattr(mod, "EXPORTS", [])
        for export in exports:
            if not hasattr(mod, export):
                continue
            value = getattr(mod, export)
            if callable(value):
                arity = len(inspect.signature(value).parameters)
                env.define(
                    export,
                    PeriodBuiltIn(
                        export,
                        arity,
                        lambda args, span, fn=value: fn(*args),
                    ),
                )
            else:
                env.define(export, value)
        return env

    def _load_file_module(self, path: Path) -> Environment:
        from .lexer import Lexer
        from .parser import Parser
        from .semantic import SemanticChecker

        source = path.read_text(encoding="utf-8")
        lexer = Lexer(source, str(path))
        tokens = lexer.scan()
        diagnostics = list(lexer.diagnostics)

        parser = Parser(tokens, source, str(path))
        program = parser.parse()
        diagnostics.extend(parser.diagnostics)

        if diagnostics:
            messages = "\n".join(f"  {d.span.line}:{d.span.col_start}: {d.message}" for d in diagnostics)
            raise RuntimeError(f"Errors in module '{path}':\n{messages}", SourceSpan(0, 0, 0))

        checker = SemanticChecker()
        module_diagnostics = checker.check(program, str(path))
        if module_diagnostics:
            messages = "\n".join(f"  {d.span.line}:{d.span.col_start}: {d.message}" for d in module_diagnostics)
            raise RuntimeError(f"Errors in module '{path}':\n{messages}", SourceSpan(0, 0, 0))

        module_interpreter = Interpreter()
        module_interpreter.filename = str(path)
        module_interpreter.interpret(program)
        return module_interpreter.environment

    def _assign_target(self, target: ast.Expr, value: Any, span: SourceSpan):
        if isinstance(target, ast.VariableExpr):
            self.environment.set(target.name, value, span)
            return
        if isinstance(target, ast.PropertyExpr):
            obj = self._evaluate(target.object)
            if isinstance(obj, PeriodInstance):
                obj.fields[target.name] = value
                return
            raise RuntimeError(f"Cannot set property on {self._type_name(obj, span)}.", target.object.span)
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
        if isinstance(expr, ast.PropertyExpr):
            return self._eval_property(expr)
        if isinstance(expr, ast.NewExpr):
            return self._eval_new(expr)
        if isinstance(expr, ast.TellExpr):
            return self._eval_tell(expr)
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
            for param, param_type, arg in zip(decl.parameters, decl.parameter_types, arguments):
                self._check_type(arg, param_type, expr.span)
                env.define(param, arg)
            try:
                self.execute_block(decl.body, env)
            except ReturnValue as ret:
                self._check_type(ret.value, decl.return_type, expr.span)
                return ret.value
            self._check_type(None, decl.return_type, expr.span)
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

    def _eval_property(self, expr: ast.PropertyExpr) -> Any:
        obj = self._evaluate(expr.object)
        if isinstance(obj, PeriodInstance):
            if expr.name in obj.fields:
                return obj.fields[expr.name]
            if expr.name in obj.klass.methods:
                method = obj.klass.methods[expr.name]
                return PeriodFunction(method, self.environment)
            raise RuntimeError(f"Instance of {obj.klass.name} has no property '{expr.name}'.", expr.span)
        raise RuntimeError(f"Cannot access property on {self._type_name(obj, expr.span)}.", expr.object.span)

    def _eval_new(self, expr: ast.NewExpr) -> Any:
        klass = self._evaluate(expr.class_expr)
        if not isinstance(klass, PeriodClass):
            raise RuntimeError(f"Cannot create new instance of {self._type_name(klass, expr.class_expr.span)}.", expr.class_expr.span)

        instance = PeriodInstance(klass)
        if klass.init is not None:
            init = klass.init
            arguments = [self._evaluate(arg) for arg in expr.arguments]
            if len(init.parameters) != len(arguments):
                raise RuntimeError(
                    f"Init of '{klass.name}' expects {len(init.parameters)} argument(s) but got {len(arguments)}.",
                    expr.span,
                )
            env = Environment(self.environment)
            env.define("this", instance)
            for param, param_type, arg in zip(init.parameters, init.parameter_types, arguments):
                self._check_type(arg, param_type, expr.span)
                env.define(param, arg)
            try:
                self.execute_block(init.body, env)
            except ReturnValue:
                pass
        return instance

    def _eval_tell(self, expr: ast.TellExpr) -> Any:
        obj = self._evaluate(expr.object)
        if not isinstance(obj, PeriodInstance):
            raise RuntimeError(f"Cannot send a message to {self._type_name(obj, expr.span)}.", expr.object.span)

        method = obj.klass.methods.get(expr.method)
        if method is None:
            raise RuntimeError(f"Class {obj.klass.name} has no method '{expr.method}'.", expr.span)

        arguments = [self._evaluate(arg) for arg in expr.arguments]
        if len(method.parameters) != len(arguments):
            raise RuntimeError(
                f"Method '{expr.method}' expects {len(method.parameters)} argument(s) but got {len(arguments)}.",
                expr.span,
            )
        env = Environment(self.environment)
        env.define("this", obj)
        for param, param_type, arg in zip(method.parameters, method.parameter_types, arguments):
            self._check_type(arg, param_type, expr.span)
            env.define(param, arg)
        try:
            self.execute_block(method.body, env)
        except ReturnValue as ret:
            self._check_type(ret.value, method.return_type, expr.span)
            return ret.value
        return None

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

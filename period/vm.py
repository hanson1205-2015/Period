"""Stack-based bytecode virtual machine for a subset of Period.

This is the "fast path" for numeric/loop-heavy code.  When the VM cannot
compile a program it raises VMUnsupportedError so the caller can fall back to
the tree-walking interpreter.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, List, Optional, Tuple

from . import ast_nodes as ast


class VMUnsupportedError(Exception):
    """Raised when the VM cannot compile a particular AST node."""

    def __init__(self, message: str, node: ast.Node):
        super().__init__(message)
        self.node = node


class VMRuntimeError(Exception):
    """Runtime error produced while executing bytecode."""


# ---------------------------------------------------------------------------
# Bytecode opcodes
# ---------------------------------------------------------------------------
OP_RETURN = 0
OP_CONSTANT = 1
OP_NEGATE = 2
OP_ADD = 3
OP_SUB = 4
OP_MUL = 5
OP_DIV = 6
OP_MOD = 7
OP_NOT = 8
OP_EQUAL = 9
OP_GREATER = 10
OP_LESS = 11
OP_TRUE = 12
OP_FALSE = 13
OP_NOTHING = 14
OP_POP = 15
OP_GET_LOCAL = 16
OP_SET_LOCAL = 17
OP_DEFINE_GLOBAL = 18
OP_GET_GLOBAL = 19
OP_SET_GLOBAL = 20
OP_PRINT = 21
OP_JUMP = 22
OP_JUMP_IF_FALSE = 23
OP_LOOP = 24
OP_CALL = 25
OP_GREATER_EQUAL = 26
OP_LESS_EQUAL = 27
OP_NOT_EQUAL = 28
OP_POWER = 29


@dataclass
class FunctionObj:
    """A compiled Period function."""

    name: str
    arity: int
    chunk: "Chunk"

    def __repr__(self) -> str:
        return f"<function {self.name}>"


@dataclass
class Chunk:
    """A block of bytecode plus its constant pool."""

    code: List[int] = field(default_factory=list)
    constants: List[Any] = field(default_factory=list)
    lines: List[int] = field(default_factory=list)

    def write(self, byte: int, line: int = 0) -> None:
        self.code.append(byte)
        self.lines.append(line)

    def add_constant(self, value: Any) -> int:
        self.constants.append(value)
        return len(self.constants) - 1

    def emit_constant(self, value: Any, line: int = 0) -> None:
        idx = self.add_constant(value)
        self.write(OP_CONSTANT, line)
        self.write(idx, line)


@dataclass
class CallFrame:
    function: FunctionObj
    ip: int = 0
    slot_base: int = 0
    return_slot: int = 0


# ---------------------------------------------------------------------------
# Compiler
# ---------------------------------------------------------------------------
class _Local:
    """A local variable in a function scope."""

    __slots__ = ("name", "depth")

    def __init__(self, name: str, depth: int):
        self.name = name
        self.depth = depth


class _Compiler:
    """Single-pass compiler from AST to bytecode."""

    def __init__(self, name: str = "<script>", arity: int = 0):
        self.chunk = Chunk()
        self.name = name
        self.arity = arity
        self.locals: List[_Local] = []
        self.scope_depth = 0
        # Reserve slots for parameters.
        self.next_local_slot = 0

    def _add_local(self, name: str) -> int:
        slot = len(self.locals)
        self.locals.append(_Local(name, self.scope_depth))
        return slot

    def _resolve_local(self, name: str) -> Optional[int]:
        for i in range(len(self.locals) - 1, -1, -1):
            if self.locals[i].name == name:
                return i
        return None

    def _emit(self, byte: int) -> None:
        self.chunk.write(byte)

    def _emit_two(self, op: int, operand: int) -> None:
        self._emit(op)
        self._emit(operand)

    def _emit_u16(self, value: int) -> None:
        self._emit((value >> 8) & 0xFF)
        self._emit(value & 0xFF)

    def _emit_constant(self, value: Any) -> None:
        self.chunk.emit_constant(value)

    def _emit_jump(self, op: int) -> int:
        self._emit(op)
        self._emit_u16(0xFFFF)
        return len(self.chunk.code) - 2

    def _patch_jump(self, offset: int) -> None:
        jump = len(self.chunk.code) - offset - 2
        if jump > 0xFFFF:
            raise VMUnsupportedError("Jump too large.", ast.Program(ast.SourceSpan(0, 0, 0)))
        self.chunk.code[offset] = (jump >> 8) & 0xFF
        self.chunk.code[offset + 1] = jump & 0xFF

    def _emit_loop(self, loop_start: int) -> None:
        self._emit(OP_LOOP)
        offset = len(self.chunk.code) - loop_start + 2
        if offset > 0xFFFF:
            raise VMUnsupportedError("Loop too large.", ast.Program(ast.SourceSpan(0, 0, 0)))
        self._emit_u16(offset)

    def compile(self, program: ast.Program) -> FunctionObj:
        for stmt in program.statements:
            self._compile_stmt(stmt)
        self._emit(OP_RETURN)
        return FunctionObj(self.name, self.arity, self.chunk)

    # Statements --------------------------------------------------------------
    def _compile_stmt(self, stmt: ast.Stmt) -> None:
        if isinstance(stmt, ast.ExpressionStmt):
            self._compile_expr(stmt.expression)
            self._emit(OP_POP)
            return

        if isinstance(stmt, ast.LetStmt):
            self._compile_expr(stmt.initializer)
            if self.scope_depth == 0:
                idx = self._global_name(stmt.name)
                self._emit_two(OP_DEFINE_GLOBAL, idx)
            else:
                self._add_local(stmt.name)
            return

        if isinstance(stmt, ast.SetStmt):
            self._compile_expr(stmt.value)
            target = stmt.target
            if isinstance(target, ast.VariableExpr):
                slot = self._resolve_local(target.name)
                if slot is not None:
                    self._emit_two(OP_SET_LOCAL, slot)
                else:
                    idx = self._global_name(target.name)
                    self._emit_two(OP_SET_GLOBAL, idx)
                return
            raise VMUnsupportedError(
                f"Assignment target {type(target).__name__} not supported by VM.", stmt
            )

        if isinstance(stmt, ast.ShowStmt):
            self._compile_expr(stmt.expression)
            self._emit(OP_PRINT)
            return

        if isinstance(stmt, ast.IfStmt):
            self._compile_expr(stmt.condition)
            then_jump = self._emit_jump(OP_JUMP_IF_FALSE)
            self._emit(OP_POP)  # discard truthy condition
            for s in stmt.then_branch:
                self._compile_stmt(s)
            else_jump = self._emit_jump(OP_JUMP)
            self._patch_jump(then_jump)
            self._emit(OP_POP)  # discard falsey condition
            for s in stmt.else_branch:
                self._compile_stmt(s)
            self._patch_jump(else_jump)
            return

        if isinstance(stmt, ast.WhileStmt):
            loop_start = len(self.chunk.code)
            self._compile_expr(stmt.condition)
            exit_jump = self._emit_jump(OP_JUMP_IF_FALSE)
            self._emit(OP_POP)
            for s in stmt.body:
                self._compile_stmt(s)
            self._emit_loop(loop_start)
            self._patch_jump(exit_jump)
            self._emit(OP_POP)
            return

        if isinstance(stmt, ast.ForStmt):
            self._compile_for(stmt)
            return

        if isinstance(stmt, ast.ReturnStmt):
            if stmt.value is not None:
                self._compile_expr(stmt.value)
            else:
                self._emit(OP_NOTHING)
            self._emit(OP_RETURN)
            return

        if isinstance(stmt, ast.DefineStmt):
            self._compile_function(stmt)
            return

        if isinstance(stmt, ast.BlockStmt):
            self._begin_scope()
            for s in stmt.statements:
                self._compile_stmt(s)
            self._end_scope()
            return

        raise VMUnsupportedError(
            f"Statement {type(stmt).__name__} not supported by VM.", stmt
        )

    def _compile_for(self, stmt: ast.ForStmt) -> None:
        """Compile ``for <var> in range with n repeat:`` loops natively."""
        if not self._is_range_call(stmt.iterable):
            raise VMUnsupportedError(
                "VM only supports 'for ... in range with ...' loops.", stmt
            )

        range_call = stmt.iterable
        args = range_call.arguments
        if len(args) == 1:
            start_expr: Optional[ast.Expr] = ast.NumberLiteral(span=stmt.span, value=0)
            stop_expr = args[0]
            step_expr: Optional[ast.Expr] = ast.NumberLiteral(span=stmt.span, value=1)
        elif len(args) == 2:
            start_expr, stop_expr = args
            step_expr = ast.NumberLiteral(span=stmt.span, value=1)
        elif len(args) == 3:
            start_expr, stop_expr, step_expr = args
        else:
            raise VMUnsupportedError("range expects 1 to 3 arguments.", stmt)

        self._begin_scope()
        # let var = start
        self._compile_expr(start_expr)
        self._add_local(stmt.variable)

        # loop: condition var < stop (or > for negative step)
        loop_start = len(self.chunk.code)
        self._emit_two(OP_GET_LOCAL, self._resolve_local(stmt.variable))
        self._compile_expr(stop_expr)
        self._emit(OP_LESS)
        exit_jump = self._emit_jump(OP_JUMP_IF_FALSE)
        self._emit(OP_POP)

        for s in stmt.body:
            self._compile_stmt(s)

        # var += step
        self._emit_two(OP_GET_LOCAL, self._resolve_local(stmt.variable))
        self._compile_expr(step_expr)
        self._emit(OP_ADD)
        self._emit_two(OP_SET_LOCAL, self._resolve_local(stmt.variable))
        self._emit(OP_POP)

        self._emit_loop(loop_start)
        self._patch_jump(exit_jump)
        self._emit(OP_POP)
        self._end_scope()

    @staticmethod
    def _is_range_call(expr: ast.Expr) -> bool:
        return (
            isinstance(expr, ast.CallExpr)
            and isinstance(expr.callee, ast.VariableExpr)
            and expr.callee.name == "range"
        )

    def _compile_function(self, stmt: ast.DefineStmt) -> None:
        if self.scope_depth != 0:
            raise VMUnsupportedError("Nested functions are not supported by VM.", stmt)

        compiler = _Compiler(stmt.name, len(stmt.parameters))
        compiler.scope_depth = 1
        for param in stmt.parameters:
            compiler.locals.append(_Local(param, 1))
        for body_stmt in stmt.body:
            compiler._compile_stmt(body_stmt)
        # Implicit return nothing.
        compiler._emit(OP_NOTHING)
        compiler._emit(OP_RETURN)
        function = FunctionObj(stmt.name, len(stmt.parameters), compiler.chunk)

        self._emit_constant(function)
        idx = self._global_name(stmt.name)
        self._emit_two(OP_DEFINE_GLOBAL, idx)

    def _begin_scope(self) -> None:
        self.scope_depth += 1

    def _end_scope(self) -> None:
        self.scope_depth -= 1
        while self.locals and self.locals[-1].depth > self.scope_depth:
            self._emit(OP_POP)
            self.locals.pop()

    # Expressions -------------------------------------------------------------
    def _compile_expr(self, expr: ast.Expr) -> None:
        if isinstance(expr, ast.NumberLiteral):
            self._emit_constant(expr.value)
            return

        if isinstance(expr, ast.BooleanLiteral):
            self._emit(OP_TRUE if expr.value else OP_FALSE)
            return

        if isinstance(expr, ast.NothingLiteral):
            self._emit(OP_NOTHING)
            return

        if isinstance(expr, ast.VariableExpr):
            slot = self._resolve_local(expr.name)
            if slot is not None:
                self._emit_two(OP_GET_LOCAL, slot)
            else:
                idx = self._global_name(expr.name)
                self._emit_two(OP_GET_GLOBAL, idx)
            return

        if isinstance(expr, ast.UnaryExpr):
            self._compile_expr(expr.operand)
            if expr.operator == "-":
                self._emit(OP_NEGATE)
            elif expr.operator == "not":
                self._emit(OP_NOT)
            else:
                raise VMUnsupportedError(f"Unary operator '{expr.operator}'.", expr)
            return

        if isinstance(expr, ast.BinaryExpr):
            self._compile_binary(expr)
            return

        if isinstance(expr, ast.CallExpr):
            self._compile_call(expr)
            return

        raise VMUnsupportedError(
            f"Expression {type(expr).__name__} not supported by VM.", expr
        )

    def _compile_binary(self, expr: ast.BinaryExpr) -> None:
        op = expr.operator

        if op == "and":
            self._compile_expr(expr.left)
            jump = self._emit_jump(OP_JUMP_IF_FALSE)
            self._emit(OP_POP)
            self._compile_expr(expr.right)
            self._patch_jump(jump)
            return

        if op == "or":
            self._compile_expr(expr.left)
            jump_if_false = self._emit_jump(OP_JUMP_IF_FALSE)
            self._emit(OP_POP)
            self._compile_expr(expr.right)
            jump_to_end = self._emit_jump(OP_JUMP)
            self._patch_jump(jump_if_false)
            self._patch_jump(jump_to_end)
            return

        self._compile_expr(expr.left)
        self._compile_expr(expr.right)

        opcodes = {
            "+": OP_ADD,
            "-": OP_SUB,
            "*": OP_MUL,
            "/": OP_DIV,
            "%": OP_MOD,
            "**": OP_POWER,
            "==": OP_EQUAL,
            "!=": OP_NOT_EQUAL,
            "<": OP_LESS,
            ">": OP_GREATER,
            "<=": OP_LESS_EQUAL,
            ">=": OP_GREATER_EQUAL,
        }
        if op in opcodes:
            self._emit(opcodes[op])
            return

        raise VMUnsupportedError(f"Binary operator '{op}' not supported by VM.", expr)

    def _compile_call(self, expr: ast.CallExpr) -> None:
        # Only direct variable/function calls for now.
        if isinstance(expr.callee, ast.VariableExpr):
            slot = self._resolve_local(expr.callee.name)
            if slot is not None:
                self._emit_two(OP_GET_LOCAL, slot)
            else:
                idx = self._global_name(expr.callee.name)
                self._emit_two(OP_GET_GLOBAL, idx)
        else:
            raise VMUnsupportedError(
                "Only direct function calls are supported by VM.", expr
            )

        if len(expr.arguments) > 255:
            raise VMUnsupportedError("Too many arguments.", expr)

        for arg in expr.arguments:
            self._compile_expr(arg)
        self._emit_two(OP_CALL, len(expr.arguments))

    # Globals -----------------------------------------------------------------
    def _global_name(self, name: str) -> int:
        # Globals are stored in the script function's constant pool as strings.
        # We reuse OP_CONSTANT's constant pool by adding the name string.
        return self.chunk.add_constant(name)


# ---------------------------------------------------------------------------
# Virtual machine
# ---------------------------------------------------------------------------
class VM:
    """Execute Period bytecode."""

    __slots__ = ("stack", "frames", "globals", "output")

    def __init__(self):
        self.stack: List[Any] = []
        self.frames: List[CallFrame] = []
        self.globals: dict = {}
        self.output: List[str] = []

    def interpret(self, function: FunctionObj) -> List[str]:
        self.frames.append(CallFrame(function, 0, 0))
        self._run()
        return self.output

    def _read_byte(self, frame: CallFrame) -> int:
        byte = frame.function.chunk.code[frame.ip]
        frame.ip += 1
        return byte

    def _read_u16(self, frame: CallFrame) -> int:
        high = self._read_byte(frame)
        low = self._read_byte(frame)
        return (high << 8) | low

    def _peek(self, distance: int = 0) -> Any:
        return self.stack[-1 - distance]

    def _push(self, value: Any) -> None:
        self.stack.append(value)

    def _pop(self) -> Any:
        return self.stack.pop()

    def _is_truthy(self, value: Any) -> bool:
        if value is None:
            return False
        if isinstance(value, bool):
            return value
        if isinstance(value, (int, float)):
            return value != 0
        if isinstance(value, str):
            return len(value) > 0
        return True

    def _binary_op(self, op: str) -> None:
        b = self._pop()
        a = self._pop()
        if op == "+":
            self._push(a + b)
        elif op == "-":
            self._push(a - b)
        elif op == "*":
            self._push(a * b)
        elif op == "/":
            if b == 0:
                raise VMRuntimeError("Division by zero.")
            self._push(a / b)
        elif op == "%":
            if b == 0:
                raise VMRuntimeError("Modulo by zero.")
            self._push(a % b)
        elif op == "**":
            self._push(a ** b)
        elif op == "==":
            self._push(a == b)
        elif op == "!=":
            self._push(a != b)
        elif op == "<":
            self._push(a < b)
        elif op == ">":
            self._push(a > b)
        elif op == "<=":
            self._push(a <= b)
        elif op == ">=":
            self._push(a >= b)
        else:
            raise VMRuntimeError(f"Unknown binary operator '{op}'.")

    def _run(self) -> None:
        stack = self.stack
        frames = self.frames
        globals_ = self.globals
        output = self.output

        frame = frames[-1]
        code = frame.function.chunk.code
        constants = frame.function.chunk.constants
        ip = frame.ip
        slot_base = frame.slot_base

        while True:
            opcode = code[ip]
            ip += 1

            if opcode == OP_RETURN:
                result = stack.pop()
                return_slot = frame.return_slot
                frames.pop()
                if not frames:
                    stack.append(result)
                    return
                frame = frames[-1]
                code = frame.function.chunk.code
                constants = frame.function.chunk.constants
                ip = frame.ip
                slot_base = frame.slot_base
                del stack[return_slot:]
                stack.append(result)
                continue

            if opcode == OP_CONSTANT:
                idx = code[ip]
                ip += 1
                stack.append(constants[idx])
                continue

            if opcode == OP_TRUE:
                stack.append(True)
                continue
            if opcode == OP_FALSE:
                stack.append(False)
                continue
            if opcode == OP_NOTHING:
                stack.append(None)
                continue
            if opcode == OP_POP:
                stack.pop()
                continue

            if opcode == OP_GET_LOCAL:
                slot = code[ip]
                ip += 1
                stack.append(stack[slot_base + slot])
                continue
            if opcode == OP_SET_LOCAL:
                slot = code[ip]
                ip += 1
                stack[slot_base + slot] = stack[-1]
                continue

            if opcode == OP_DEFINE_GLOBAL:
                idx = code[ip]
                ip += 1
                name = constants[idx]
                globals_[name] = stack.pop()
                continue
            if opcode == OP_GET_GLOBAL:
                idx = code[ip]
                ip += 1
                name = constants[idx]
                if name not in globals_:
                    raise VMRuntimeError(f"Undefined variable '{name}'.")
                stack.append(globals_[name])
                continue
            if opcode == OP_SET_GLOBAL:
                idx = code[ip]
                ip += 1
                name = constants[idx]
                if name not in globals_:
                    raise VMRuntimeError(f"Undefined variable '{name}'.")
                globals_[name] = stack[-1]
                continue

            if opcode == OP_NEGATE:
                stack.append(-stack.pop())
                continue
            if opcode == OP_NOT:
                value = stack.pop()
                if value is None:
                    stack.append(True)
                elif isinstance(value, bool):
                    stack.append(not value)
                elif isinstance(value, (int, float)):
                    stack.append(value == 0)
                elif isinstance(value, str):
                    stack.append(len(value) == 0)
                else:
                    stack.append(False)
                continue

            if opcode == OP_ADD:
                b = stack.pop(); a = stack.pop(); stack.append(a + b)
                continue
            if opcode == OP_SUB:
                b = stack.pop(); a = stack.pop(); stack.append(a - b)
                continue
            if opcode == OP_MUL:
                b = stack.pop(); a = stack.pop(); stack.append(a * b)
                continue
            if opcode == OP_DIV:
                b = stack.pop(); a = stack.pop()
                if b == 0:
                    raise VMRuntimeError("Division by zero.")
                stack.append(a / b)
                continue
            if opcode == OP_MOD:
                b = stack.pop(); a = stack.pop()
                if b == 0:
                    raise VMRuntimeError("Modulo by zero.")
                stack.append(a % b)
                continue
            if opcode == OP_POWER:
                b = stack.pop(); a = stack.pop(); stack.append(a ** b)
                continue
            if opcode == OP_EQUAL:
                b = stack.pop(); a = stack.pop(); stack.append(a == b)
                continue
            if opcode == OP_NOT_EQUAL:
                b = stack.pop(); a = stack.pop(); stack.append(a != b)
                continue
            if opcode == OP_LESS:
                b = stack.pop(); a = stack.pop(); stack.append(a < b)
                continue
            if opcode == OP_GREATER:
                b = stack.pop(); a = stack.pop(); stack.append(a > b)
                continue
            if opcode == OP_LESS_EQUAL:
                b = stack.pop(); a = stack.pop(); stack.append(a <= b)
                continue
            if opcode == OP_GREATER_EQUAL:
                b = stack.pop(); a = stack.pop(); stack.append(a >= b)
                continue

            if opcode == OP_PRINT:
                value = stack.pop()
                if value is None:
                    text = "nothing"
                elif isinstance(value, bool):
                    text = "true" if value else "false"
                else:
                    text = str(value)
                output.append(text)
                print(text)
                continue

            if opcode == OP_JUMP:
                high = code[ip]; low = code[ip + 1]; ip += 2
                ip += (high << 8) | low
                continue
            if opcode == OP_JUMP_IF_FALSE:
                high = code[ip]; low = code[ip + 1]; ip += 2
                value = stack[-1]
                falsey = (
                    value is None
                    or (isinstance(value, bool) and not value)
                    or (isinstance(value, (int, float)) and value == 0)
                    or (isinstance(value, str) and len(value) == 0)
                )
                if falsey:
                    ip += (high << 8) | low
                continue
            if opcode == OP_LOOP:
                high = code[ip]; low = code[ip + 1]; ip += 2
                ip -= (high << 8) | low
                continue

            if opcode == OP_CALL:
                arg_count = code[ip]
                ip += 1
                callee = stack[-1 - arg_count]
                if not isinstance(callee, FunctionObj):
                    raise VMRuntimeError("Can only call functions.")
                if callee.arity != arg_count:
                    raise VMRuntimeError(
                        f"Function '{callee.name}' expects {callee.arity} arguments "
                        f"but got {arg_count}."
                    )
                # Save current frame state before pushing the new one.
                frame.ip = ip
                # The callee sits just below its arguments. After removing it, the
                # first argument occupies the callee's old slot.
                callee_idx = len(stack) - arg_count - 1
                frames.append(CallFrame(callee, 0, callee_idx, callee_idx))
                stack.pop(callee_idx)
                frame = frames[-1]
                code = frame.function.chunk.code
                constants = frame.function.chunk.constants
                ip = 0
                slot_base = frame.slot_base
                continue

            raise VMRuntimeError(f"Unknown opcode {opcode}.")

    @staticmethod
    def _stringify(value: Any) -> str:
        if value is None:
            return "nothing"
        if isinstance(value, bool):
            return "true" if value else "false"
        if isinstance(value, FunctionObj):
            return str(value)
        return str(value)


def compile_program(program: ast.Program) -> FunctionObj:
    """Compile a Program AST to a top-level bytecode function."""
    compiler = _Compiler()
    return compiler.compile(program)


def run(program: ast.Program) -> Tuple[bool, List[str], str]:
    """Run *program* with the bytecode VM.

    Returns ``(success, output_lines, error_message)``.
    """
    try:
        function = compile_program(program)
    except VMUnsupportedError as exc:
        return False, [], str(exc)

    vm = VM()
    try:
        vm.interpret(function)
    except VMRuntimeError as exc:
        return False, vm.output, str(exc)
    return True, vm.output, ""

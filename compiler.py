#!/usr/bin/env python3
"""Command-line compiler and REPL for the Period programming language."""
import argparse
import os
import subprocess
import sys
from pathlib import Path

def format_diagnostics(source: str, diagnostics, filename: str = "<stdin>") -> str:
    lines = source.splitlines()
    parts = []
    for diag in diagnostics:
        span = diag.span
        line_text = lines[span.line - 1] if span.line - 1 < len(lines) else ""
        underline = " " * (span.col_start - 1) + "^" * max(1, span.col_end - span.col_start)
        parts.append(
            f"{filename}:{span.line}:{span.col_start}: {diag.severity}: {diag.message}\n"
            f"    {span.line:4d} | {line_text}\n"
            f"         | {underline}"
        )
    return "\n".join(parts)


def run_source(
    source: str,
    filename: str = "<stdin>",
    print_output: bool = True,
    use_native: bool = False,
    use_jit: bool = False,
    use_vm: bool = False,
) -> int:
    from period.lexer import Lexer
    from period.parser import Parser
    from period.semantic import SemanticChecker

    lexer = Lexer(source, filename)
    tokens = lexer.scan()

    diagnostics = list(lexer.diagnostics)

    parser = Parser(tokens, source, filename)
    program = parser.parse()
    diagnostics.extend(parser.diagnostics)

    checker = SemanticChecker()
    diagnostics.extend(checker.check(program, filename))

    if diagnostics:
        print(format_diagnostics(source, diagnostics, filename), file=sys.stderr)
        return 1

    if use_native:
        from period.c_backend import run_native
        ok, stdout, stderr = run_native(program)
        if not ok:
            print(f"native backend failed: {stderr}", file=sys.stderr)
            print("falling back to interpreter.", file=sys.stderr)
        else:
            if stdout:
                print(stdout, end="")
            return 0

    if use_jit:
        from period.jit_backend import run as run_jit
        ok, output, stderr = run_jit(program)
        if ok:
            return 0
        print(f"jit backend failed: {stderr}", file=sys.stderr)
        print("falling back to --fast.", file=sys.stderr)
        use_vm = True

    if use_vm:
        from period.py_backend import run as run_py
        from period.vm import run as run_vm
        ok, output, stderr = run_py(program)
        if ok:
            if print_output:
                for line in output:
                    print(line)
            return 0
        print(f"python backend failed: {stderr}", file=sys.stderr)
        ok, output, stderr = run_vm(program)
        if ok:
            return 0
        print(f"vm backend failed: {stderr}", file=sys.stderr)
        print("falling back to interpreter.", file=sys.stderr)

    from period.interpreter import Interpreter
    interpreter = Interpreter()
    try:
        interpreter.interpret(program, filename)
    except Exception as e:
        print(f"{filename}: runtime error: {e}", file=sys.stderr)
        return 1

    return 0


def _try_rust_backend(path: Path) -> int:
    """Run the Rust-native compiler if it is available. Returns its exit code."""
    exe_candidates = [
        Path(__file__).with_name("period-rs.exe"),
        Path(__file__).parent / "period-rs" / "target" / "release" / "period-rs.exe",
    ]
    exe = next((c for c in exe_candidates if c.exists()), None)
    if exe is None:
        return -1
    result = subprocess.run([str(exe), str(path)], capture_output=True, text=True)
    if result.returncode == 0:
        if result.stdout:
            print(result.stdout, end="")
        return 0
    # Unsupported constructs produce a non-zero exit; let the Python stack
    # report the issue and fall back gracefully.
    return result.returncode


def run_file(path: Path, use_native: bool = False, use_jit: bool = False, use_vm: bool = False) -> int:
    if not (use_native or use_jit or use_vm):
        rust_code = _try_rust_backend(path)
        if rust_code == 0:
            return 0
    source = path.read_text(encoding="utf-8")
    return run_source(source, str(path), use_native=use_native, use_jit=use_jit, use_vm=use_vm)


def run_repl():
    from period.interpreter import Interpreter
    from period.lexer import Lexer
    from period.parser import Parser

    print("Period programming language REPL.")
    print("Type a statement ending with '.' and press Enter. Use Ctrl+C or type 'exit.' to quit.")
    print()
    interpreter = Interpreter()
    buffer = []
    while True:
        try:
            prompt = "... " if buffer else ">>> "
            line = input(prompt)
        except (EOFError, KeyboardInterrupt):
            print()
            break
        buffer.append(line)
        text = "\n".join(buffer)
        stripped = text.strip().lower()
        if stripped in {"exit.", "quit."}:
            break
        # Heuristic: a complete REPL entry must end with a period (ignoring whitespace).
        if text.rstrip().endswith("."):
            lexer = Lexer(text, "<repl>")
            tokens = lexer.scan()
            diagnostics = list(lexer.diagnostics)
            parser = Parser(tokens, text, "<repl>")
            program = parser.parse()
            diagnostics.extend(parser.diagnostics)
            if diagnostics:
                print(format_diagnostics(text, diagnostics, "<repl>"))
                buffer.clear()
                continue
            try:
                interpreter.interpret(program)
            except Exception as e:
                print(f"runtime error: {e}")
            buffer.clear()


def main():
    argparser = argparse.ArgumentParser(
        prog="period",
        description="Compile and run Period source files, or start an interactive REPL.",
    )
    argparser.add_argument("file", nargs="?", help="Path to a .period source file.")
    argparser.add_argument(
        "--lsp",
        action="store_true",
        help="Start the Language Server Protocol (LSP) service on stdin/stdout.",
    )
    argparser.add_argument(
        "--stdio",
        action="store_true",
        help="Use stdin/stdout for LSP (used by VS Code; same as --lsp).",
    )
    argparser.add_argument(
        "--version",
        action="version",
        version="Period 1.1.0",
    )
    argparser.add_argument(
        "--native",
        action="store_true",
        help="Compile the program to C and run the native executable (numeric subset only).",
    )
    argparser.add_argument(
        "--jit",
        action="store_true",
        help="JIT-compile the program with Numba and run the machine code (numeric subset only).",
    )
    argparser.add_argument(
        "--fast",
        action="store_true",
        help="Run the program on the Python fast path / bytecode VM (numeric subset only).",
    )
    args = argparser.parse_args()

    if args.lsp or args.stdio:
        from period.lsp_server import LSPServer
        server = LSPServer()
        server.run()
        return 0

    if args.file:
        path = Path(args.file)
        if not path.exists():
            print(f"error: file not found: {path}", file=sys.stderr)
            return 1
        return run_file(path, use_native=args.native, use_jit=args.jit, use_vm=args.fast)

    run_repl()
    return 0


if __name__ == "__main__":
    sys.exit(main())

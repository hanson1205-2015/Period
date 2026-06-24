#!/usr/bin/env python3
"""Command-line compiler and REPL for the Period programming language."""
import argparse
import os
import sys
from pathlib import Path

from period.interpreter import Interpreter
from period.lexer import Lexer
from period.lsp_server import LSPServer
from period.parser import Parser


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


def run_source(source: str, filename: str = "<stdin>", print_output: bool = True) -> int:
    try:
        lexer = Lexer(source, filename)
        tokens = lexer.scan()
    except Exception as e:
        print(f"{filename}: fatal lexer error: {e}", file=sys.stderr)
        return 1

    parser = Parser(tokens, source, filename)
    program = parser.parse()

    if parser.diagnostics:
        print(format_diagnostics(source, parser.diagnostics, filename), file=sys.stderr)
        return 1

    interpreter = Interpreter()
    try:
        interpreter.interpret(program)
    except Exception as e:
        print(f"{filename}: runtime error: {e}", file=sys.stderr)
        return 1

    return 0


def run_file(path: Path) -> int:
    source = path.read_text(encoding="utf-8")
    return run_source(source, str(path))


def run_repl():
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
            try:
                tokens = lexer.scan()
            except Exception as e:
                print(f"lexer error: {e}")
                buffer.clear()
                continue
            parser = Parser(tokens, text, "<repl>")
            program = parser.parse()
            if parser.diagnostics:
                print(format_diagnostics(text, parser.diagnostics, "<repl>"))
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
        version="Period 0.0.1",
    )
    args = argparser.parse_args()

    if args.lsp or args.stdio:
        server = LSPServer()
        server.run()
        return 0

    if args.file:
        path = Path(args.file)
        if not path.exists():
            print(f"error: file not found: {path}", file=sys.stderr)
            return 1
        return run_file(path)

    run_repl()
    return 0


if __name__ == "__main__":
    sys.exit(main())

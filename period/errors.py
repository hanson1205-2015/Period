"""Error handling for the Period programming language."""
from dataclasses import dataclass
from typing import Optional


@dataclass(frozen=True)
class SourceSpan:
    """A span of source code identified by line and column offsets."""
    line: int
    col_start: int
    col_end: int

    def __str__(self) -> str:
        if self.col_start == self.col_end:
            return f"line {self.line}, column {self.col_start}"
        return f"line {self.line}, columns {self.col_start}-{self.col_end}"


class PeriodError(Exception):
    """Base class for all Period errors carrying source location."""

    def __init__(self, message: str, span: Optional[SourceSpan] = None):
        super().__init__(message)
        self.message = message
        self.span = span

    def __str__(self) -> str:
        if self.span is None:
            return self.message
        return f"[{self.span}] {self.message}"


class LexerError(PeriodError):
    """Error raised by the lexer."""


class ParseError(PeriodError):
    """Error raised by the parser."""


class RuntimeError(PeriodError):
    """Error raised while executing Period code."""


class Diagnostic:
    """A diagnostic message produced during static analysis."""

    def __init__(self, message: str, span: SourceSpan, severity: str = "error"):
        self.message = message
        self.span = span
        self.severity = severity  # 'error', 'warning', 'info'

    def __str__(self) -> str:
        return f"[{self.severity.upper()}] {self.span}: {self.message}"

    def to_dict(self):
        return {
            "message": self.message,
            "line": self.span.line,
            "col_start": self.span.col_start,
            "col_end": self.span.col_end,
            "severity": self.severity,
        }

"""Lexer for the Period programming language."""
from dataclasses import dataclass
from enum import Enum, auto
from typing import List, Optional

from .errors import Diagnostic, SourceSpan


class TokenType(Enum):
    # Literals
    NUMBER = auto()
    STRING = auto()
    IDENTIFIER = auto()
    TRUE = auto()
    FALSE = auto()
    NOTHING = auto()

    # Keywords
    LET = auto()
    BE = auto()
    SET = auto()
    TO = auto()
    SHOW = auto()
    IF = auto()
    THEN = auto()
    OTHERWISE = auto()
    WHILE = auto()
    REPEAT = auto()
    DEFINE = auto()
    WITH = auto()
    RETURN = auto()
    RETURNS = auto()
    AND = auto()
    OR = auto()
    NOT = auto()
    INPUT = auto()

    # Class / object keywords
    CLASS = auto()
    INIT = auto()
    THIS = auto()
    NEW = auto()
    TELL = auto()
    OF = auto()
    THE = auto()

    # Operators / punctuation
    PLUS = auto()
    MINUS = auto()
    STAR = auto()
    SLASH = auto()
    PERCENT = auto()
    POWER = auto()
    EQUAL = auto()
    EQUAL_EQUAL = auto()
    BANG_EQUAL = auto()
    LESS = auto()
    GREATER = auto()
    LESS_EQUAL = auto()
    GREATER_EQUAL = auto()
    LPAREN = auto()
    RPAREN = auto()
    LBRACKET = auto()
    RBRACKET = auto()
    LBRACE = auto()
    RBRACE = auto()
    COMMA = auto()
    COLON = auto()
    DOT = auto()

    # Indentation
    INDENT = auto()
    DEDENT = auto()

    # Special
    NEWLINE = auto()
    EOF = auto()
    COMMENT = auto()
    ERROR = auto()


KEYWORDS = {
    "let": TokenType.LET,
    "be": TokenType.BE,
    "set": TokenType.SET,
    "to": TokenType.TO,
    "show": TokenType.SHOW,
    "if": TokenType.IF,
    "then": TokenType.THEN,
    "otherwise": TokenType.OTHERWISE,
    "while": TokenType.WHILE,
    "repeat": TokenType.REPEAT,
    "define": TokenType.DEFINE,
    "with": TokenType.WITH,
    "return": TokenType.RETURN,
    "returns": TokenType.RETURNS,
    "and": TokenType.AND,
    "or": TokenType.OR,
    "not": TokenType.NOT,
    "true": TokenType.TRUE,
    "false": TokenType.FALSE,
    "nothing": TokenType.NOTHING,
    "input": TokenType.INPUT,
    "class": TokenType.CLASS,
    "init": TokenType.INIT,
    "this": TokenType.THIS,
    "new": TokenType.NEW,
    "tell": TokenType.TELL,
    "of": TokenType.OF,
    "the": TokenType.THE,
}


@dataclass(frozen=True)
class Token:
    type: TokenType
    value: object
    span: SourceSpan
    lexeme: str

    def __str__(self) -> str:
        return f"{self.type.name}({self.lexeme!r}) at {self.span}"


class Lexer:
    """Transform Period source code into a sequence of tokens."""

    def __init__(self, source: str, filename: str = "<stdin>"):
        self.source = source
        self.filename = filename
        self.tokens: List[Token] = []
        self.diagnostics: List[Diagnostic] = []
        self.start = 0
        self.current = 0
        self.line = 1
        self.col_start = 1

    def scan(self) -> List[Token]:
        """Scan all tokens from the source, reporting lexer errors."""
        while not self._is_at_end():
            self.start = self.current
            self.col_start = self._current_col()
            self._scan_token()
        self.tokens.append(
            Token(TokenType.EOF, None, SourceSpan(self.line, self._current_col(), self._current_col()), "")
        )
        self._process_indentation()
        return self.tokens

    def _is_at_end(self) -> bool:
        return self.current >= len(self.source)

    def _current_col(self) -> int:
        # column is 1-based character index on current line
        line_start = self.source.rfind("\n", 0, self.current)
        if line_start == -1:
            return self.current + 1
        return self.current - line_start

    def _span(self, length: int = 0) -> SourceSpan:
        col_end = self.col_start + max(length, self.current - self.start)
        return SourceSpan(self.line, self.col_start, col_end)

    def _advance(self) -> str:
        char = self.source[self.current]
        self.current += 1
        return char

    def _peek(self, offset: int = 0) -> str:
        pos = self.current + offset
        if pos >= len(self.source):
            return "\0"
        return self.source[pos]

    def _match(self, expected: str) -> bool:
        if self._is_at_end():
            return False
        if self.source[self.current] != expected:
            return False
        self.current += 1
        return True

    def _add_token(self, token_type: TokenType, value: object = None):
        lexeme = self.source[self.start : self.current]
        self.tokens.append(Token(token_type, value, self._span(), lexeme))

    def _scan_token(self):
        char = self._advance()

        if char in " \r\t":
            return  # ignore whitespace

        if char == "\n":
            self._add_token(TokenType.NEWLINE, None)
            self.line += 1
            return

        if char == "-" and self._peek() == "-":
            self._comment()
            return

        if char == ".":
            self._add_token(TokenType.DOT, None)
            return

        if char == "+":
            self._add_token(TokenType.PLUS)
            return
        if char == "-":
            self._add_token(TokenType.MINUS)
            return
        if char == "*":
            if self._match("*"):
                self._add_token(TokenType.POWER)
            else:
                self._add_token(TokenType.STAR)
            return
        if char == "/":
            self._add_token(TokenType.SLASH)
            return
        if char == "%":
            self._add_token(TokenType.PERCENT)
            return
        if char == "(":
            self._add_token(TokenType.LPAREN)
            return
        if char == ")":
            self._add_token(TokenType.RPAREN)
            return
        if char == "[":
            self._add_token(TokenType.LBRACKET)
            return
        if char == "]":
            self._add_token(TokenType.RBRACKET)
            return
        if char == "{":
            self._add_token(TokenType.LBRACE)
            return
        if char == "}":
            self._add_token(TokenType.RBRACE)
            return
        if char == ",":
            self._add_token(TokenType.COMMA)
            return
        if char == ":":
            self._add_token(TokenType.COLON)
            return

        if char == "=":
            if self._match("="):
                self._add_token(TokenType.EQUAL_EQUAL)
            else:
                self._add_token(TokenType.EQUAL)
            return
        if char == "!":
            if self._match("="):
                self._add_token(TokenType.BANG_EQUAL)
            else:
                self._error(f"Unexpected character '!'; did you mean '!='?")
            return
        if char == "<":
            if self._match("="):
                self._add_token(TokenType.LESS_EQUAL)
            else:
                self._add_token(TokenType.LESS)
            return
        if char == ">":
            if self._match("="):
                self._add_token(TokenType.GREATER_EQUAL)
            else:
                self._add_token(TokenType.GREATER)
            return

        if char == '"':
            self._string()
            return

        if char.isdigit():
            self._number()
            return

        if char.isalpha() or char == "_":
            self._identifier()
            return

        self._error(f"Unexpected character '{char}'.")

    def _comment(self):
        while self._peek() != "\n" and not self._is_at_end():
            self._advance()
        self._add_token(TokenType.COMMENT, self.source[self.start : self.current])

    def _string(self):
        start_line = self.line
        start_col = self.col_start
        value_parts = []
        while self._peek() != '"' and not self._is_at_end():
            if self._peek() == "\n":
                self._advance()
                self.line += 1
                continue
            if self._peek() == "\\":
                self._advance()
                esc = self._advance()
                if esc == "n":
                    value_parts.append("\n")
                elif esc == "t":
                    value_parts.append("\t")
                elif esc == '"':
                    value_parts.append('"')
                elif esc == "\\":
                    value_parts.append("\\")
                else:
                    self._error(
                        f"Unknown escape sequence '\\{esc}'.",
                        SourceSpan(self.line, self._current_col() - 1, self._current_col()),
                    )
            else:
                value_parts.append(self._advance())

        if self._is_at_end():
            self._error(
                "Unterminated string literal.",
                SourceSpan(start_line, start_col, start_col + 1),
            )
            value = "".join(value_parts)
            self.tokens.append(Token(TokenType.STRING, value, SourceSpan(start_line, start_col, self._current_col()), self.source[self.start : self.current]))
            return

        self._advance()  # closing quote
        value = "".join(value_parts)
        self._add_token(TokenType.STRING, value)

    def _number(self):
        while self._peek().isdigit():
            self._advance()
        is_float = False
        if self._peek() == "." and self._peek(1).isdigit():
            is_float = True
            self._advance()  # consume '.'
            while self._peek().isdigit():
                self._advance()

        lexeme = self.source[self.start : self.current]
        if is_float:
            self._add_token(TokenType.NUMBER, float(lexeme))
        else:
            self._add_token(TokenType.NUMBER, int(lexeme))

    def _identifier(self):
        while self._peek().isalnum() or self._peek() == "_":
            self._advance()
        lexeme = self.source[self.start : self.current]
        token_type = KEYWORDS.get(lexeme.lower())
        if token_type is None:
            token_type = TokenType.IDENTIFIER
        self._add_token(token_type, lexeme)

    def _error(self, message: str, span: Optional[SourceSpan] = None):
        if span is None:
            span = self._span()
        self.diagnostics.append(Diagnostic(message, span, "error"))
        self._add_token(TokenType.ERROR, message)

    # -------------------------------------------------------------------------
    # Indentation handling
    # -------------------------------------------------------------------------

    def _process_indentation(self):
        """Convert NEWLINE tokens into INDENT/DEDENT based on leading whitespace."""
        processed: List[Token] = []
        stack = [0]
        i = 0
        n = len(self.tokens)

        def next_meaningful(start_idx: int) -> Optional[Token]:
            j = start_idx
            while j < n and self.tokens[j].type in (TokenType.NEWLINE, TokenType.COMMENT):
                j += 1
            if j < n and self.tokens[j].type != TokenType.EOF:
                return self.tokens[j]
            return None

        # Initial indentation check.
        first = next_meaningful(0)
        if first is not None and first.span.col_start > 1:
            indent = first.span.col_start - 1
            self.diagnostics.append(
                Diagnostic(
                    "Unexpected indentation at start of file.",
                    first.span,
                    "error",
                )
            )
            processed.append(Token(TokenType.INDENT, None, first.span, ""))
            stack.append(indent)

        while i < n:
            tok = self.tokens[i]
            if tok.type == TokenType.NEWLINE:
                processed.append(tok)
                nxt = next_meaningful(i + 1)
                if nxt is not None:
                    indent = nxt.span.col_start - 1
                    if indent > stack[-1]:
                        processed.append(Token(TokenType.INDENT, None, nxt.span, ""))
                        stack.append(indent)
                    elif indent < stack[-1]:
                        while indent < stack[-1]:
                            stack.pop()
                            processed.append(Token(TokenType.DEDENT, None, nxt.span, ""))
                        if indent != stack[-1]:
                            self.diagnostics.append(
                                Diagnostic(
                                    "Inconsistent indentation.",
                                    nxt.span,
                                    "error",
                                )
                            )
                i += 1
                continue

            processed.append(tok)
            i += 1

        # Close any remaining open blocks before the EOF token.
        if processed and processed[-1].type == TokenType.EOF:
            eof = processed.pop()
        else:
            eof = Token(TokenType.EOF, None, SourceSpan(self.line, 1, 1), "")

        while stack[-1] != 0:
            stack.pop()
            processed.append(Token(TokenType.DEDENT, None, eof.span, ""))

        processed.append(eof)
        self.tokens = processed

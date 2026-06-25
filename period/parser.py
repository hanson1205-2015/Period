"""Recursive-descent parser for Period with multi-error recovery."""
from typing import List, Optional

from . import ast_nodes as ast
from .errors import Diagnostic, ParseError, SourceSpan
from .lexer import Token, TokenType


class Parser:
    """Parse a token stream into an AST, collecting diagnostics."""

    def __init__(self, tokens: List[Token], source: str, filename: str = "<stdin>"):
        self.tokens = tokens
        self.source = source
        self.filename = filename
        self.current = 0
        self.diagnostics: List[Diagnostic] = []

    def parse(self) -> ast.Program:
        """Parse the full token stream into a Program node."""
        statements: List[ast.Stmt] = []
        while not self._is_at_end():
            stmt = self._statement()
            if stmt is not None:
                statements.append(stmt)
        span = self._peek().span
        return ast.Program(span=span, statements=statements)

    # Helpers -----------------------------------------------------------------

    def _is_at_end(self) -> bool:
        return self._peek().type == TokenType.EOF

    def _peek(self, offset: int = 0) -> Token:
        pos = self.current + offset
        if pos >= len(self.tokens):
            return self.tokens[-1]
        return self.tokens[pos]

    def _previous(self) -> Token:
        return self.tokens[self.current - 1]

    def _advance(self) -> Token:
        if not self._is_at_end():
            self.current += 1
        return self._previous()

    def _check(self, token_type: TokenType) -> bool:
        if self._is_at_end():
            return False
        return self._peek().type == token_type

    def _match(self, *types: TokenType) -> bool:
        for token_type in types:
            if self._check(token_type):
                self._advance()
                return True
        return False

    def _consume(self, token_type: TokenType, message: str) -> Optional[Token]:
        if self._check(token_type):
            return self._advance()
        if self._check(TokenType.ERROR):
            # Lexer has already reported this token; don't pile on a duplicate.
            self._advance()
            return None
        self._error(message, self._peek().span)
        return None

    def _error(self, message: str, span: SourceSpan):
        diag = Diagnostic(message, span, "error")
        self.diagnostics.append(diag)
        raise ParseError(message, span)

    def _recover(self):
        """Advance to the next synchronization point so parsing can continue."""
        self._advance()
        while not self._is_at_end():
            if self._previous().type in (TokenType.DOT, TokenType.COLON):
                return
            if self._peek().type in {
                TokenType.LET,
                TokenType.SET,
                TokenType.SHOW,
                TokenType.IF,
                TokenType.WHILE,
                TokenType.DEFINE,
                TokenType.RETURN,
                TokenType.CLASS,
                TokenType.INIT,
                TokenType.OTHERWISE,
                TokenType.DEDENT,
            }:
                return
            self._advance()

    def _span_from(self, start, end) -> SourceSpan:
        start_span = start.span if isinstance(start, Token) else start
        end_span = end.span if isinstance(end, Token) else end
        return SourceSpan(start_span.line, start_span.col_start, end_span.col_end)

    # Statements --------------------------------------------------------------

    def _statement(self) -> Optional[ast.Stmt]:
        # Skip stray newlines, comments, indents, dedents, and already-reported error tokens.
        while self._match(TokenType.NEWLINE, TokenType.COMMENT, TokenType.INDENT, TokenType.DEDENT, TokenType.ERROR):
            pass
        if self._is_at_end():
            return None

        try:
            if self._check(TokenType.LET):
                return self._let_statement()
            if self._check(TokenType.SET):
                return self._set_statement()
            if self._check(TokenType.SHOW):
                return self._show_statement()
            if self._check(TokenType.IF):
                return self._if_statement()
            if self._check(TokenType.WHILE):
                return self._while_statement()
            if self._check(TokenType.DEFINE):
                return self._define_statement()
            if self._check(TokenType.RETURN):
                return self._return_statement()
            if self._check(TokenType.CLASS):
                return self._class_statement()
            return self._expression_statement()
        except ParseError:
            self._recover()
            return None

    def _let_statement(self) -> ast.Stmt:
        start = self._advance()  # let
        name_tok = self._consume(TokenType.IDENTIFIER, "Expected a variable name after 'let'.")
        if name_tok is None:
            return None
        self._consume(TokenType.BE, "Expected 'be' after variable name in a 'let' statement.")
        init = self._expression()
        end = self._consume(TokenType.DOT, "Expected '.' at the end of a 'let' statement.")
        if end is None:
            end = self._previous()
        span = self._span_from(start, end if end else self._previous())
        return ast.LetStmt(span=span, name=name_tok.value, initializer=init)

    def _set_statement(self) -> ast.Stmt:
        start = self._advance()  # set
        target = self._expression()
        self._consume(TokenType.TO, "Expected 'to' after the target of a 'set' statement.")
        value = self._expression()
        end = self._consume(TokenType.DOT, "Expected '.' at the end of a 'set' statement.")
        if end is None:
            end = self._previous()
        span = self._span_from(start, end)
        return ast.SetStmt(span=span, target=target, value=value)

    def _show_statement(self) -> ast.Stmt:
        start = self._advance()  # show
        value = self._expression()
        end = self._consume(TokenType.DOT, "Expected '.' at the end of a 'show' statement.")
        if end is None:
            end = self._previous()
        span = self._span_from(start, end)
        return ast.ShowStmt(span=span, expression=value)

    def _if_statement(self) -> ast.Stmt:
        start = self._advance()  # if
        condition = self._expression()
        self._match(TokenType.COMMA)  # optional comma before then
        self._consume(TokenType.THEN, "Expected 'then' after the condition of an 'if' statement.")
        self._consume(TokenType.COLON, "Expected ':' after 'then'.")
        then_branch = self._block("if")

        else_branch: List[ast.Stmt] = []
        if self._match(TokenType.OTHERWISE):
            self._consume(TokenType.COLON, "Expected ':' after 'otherwise'.")
            else_branch = self._block("otherwise")

        span = self._span_from(start, self._previous())
        return ast.IfStmt(span=span, condition=condition, then_branch=then_branch, else_branch=else_branch)

    def _while_statement(self) -> ast.Stmt:
        start = self._advance()  # while
        condition = self._expression()
        self._consume(TokenType.REPEAT, "Expected 'repeat' after the condition of a 'while' statement.")
        self._consume(TokenType.COLON, "Expected ':' after 'repeat'.")
        body = self._block("while")
        span = self._span_from(start, self._previous())
        return ast.WhileStmt(span=span, condition=condition, body=body)

    def _define_statement(self) -> ast.Stmt:
        start = self._advance()  # define
        name_tok = self._consume(TokenType.IDENTIFIER, "Expected a function name after 'define'.")
        if name_tok is None:
            return None
        parameters: List[str] = []
        if self._match(TokenType.WITH):
            param = self._consume(TokenType.IDENTIFIER, "Expected a parameter name after 'with'.")
            if param:
                parameters.append(param.value)
                while self._match(TokenType.COMMA):
                    param = self._consume(TokenType.IDENTIFIER, "Expected a parameter name after ','.")
                    if param:
                        parameters.append(param.value)
        self._consume(TokenType.COLON, "Expected ':' after the function signature.")
        body = self._block("define")
        span = self._span_from(start, self._previous())
        return ast.DefineStmt(span=span, name=name_tok.value, parameters=parameters, body=body)

    def _return_statement(self) -> ast.Stmt:
        start = self._advance()  # return
        value: Optional[ast.Expr] = None
        if not self._check(TokenType.DOT):
            value = self._expression()
        end = self._consume(TokenType.DOT, "Expected '.' at the end of a 'return' statement.")
        if end is None:
            end = self._previous()
        span = self._span_from(start, end)
        return ast.ReturnStmt(span=span, value=value)

    def _class_statement(self) -> ast.Stmt:
        start = self._advance()  # class
        name_tok = self._consume(TokenType.IDENTIFIER, "Expected a class name after 'class'.")
        if name_tok is None:
            return None
        self._consume(TokenType.COLON, "Expected ':' after the class name.")
        body = self._class_body()
        span = self._span_from(start, self._previous())
        return ast.ClassStmt(span=span, name=name_tok.value, body=body)

    def _class_body(self) -> List[ast.Stmt]:
        while self._match(TokenType.NEWLINE, TokenType.COMMENT, TokenType.ERROR):
            pass
        if not self._match(TokenType.INDENT):
            self._error("Expected an indented class body.", self._peek().span)
            return []

        members: List[ast.Stmt] = []
        while not self._is_at_end() and not self._check(TokenType.DEDENT):
            while self._match(TokenType.NEWLINE, TokenType.COMMENT, TokenType.DEDENT, TokenType.ERROR):
                if self._check(TokenType.DEDENT):
                    break
            if self._is_at_end() or self._check(TokenType.DEDENT):
                break

            try:
                if self._check(TokenType.INIT):
                    members.append(self._init_member())
                elif self._check(TokenType.DEFINE):
                    members.append(self._define_statement())
                else:
                    self._error(
                        "Expected 'init' or 'define' inside a class body.",
                        self._peek().span,
                    )
            except ParseError:
                self._recover()

        while self._match(TokenType.NEWLINE, TokenType.COMMENT, TokenType.ERROR):
            pass
        self._consume(TokenType.DEDENT, "Expected the class body to end.")
        return members

    def _init_member(self) -> ast.Stmt:
        start = self._advance()  # init
        parameters: List[str] = []
        if self._match(TokenType.WITH):
            param = self._consume(TokenType.IDENTIFIER, "Expected a parameter name after 'with'.")
            if param:
                parameters.append(param.value)
                while self._match(TokenType.COMMA):
                    param = self._consume(TokenType.IDENTIFIER, "Expected a parameter name after ','.")
                    if param:
                        parameters.append(param.value)
        self._consume(TokenType.COLON, "Expected ':' after the init signature.")
        body = self._block("init")
        span = self._span_from(start, self._previous())
        return ast.InitStmt(span=span, parameters=parameters, body=body)

    def _expression_statement(self) -> ast.Stmt:
        expr = self._expression()
        end = self._consume(TokenType.DOT, "Expected '.' at the end of the statement.")
        if end is None:
            end = self._previous()
        span = self._span_from(expr.span, end)
        return ast.ExpressionStmt(span=span, expression=expr)

    def _block(self, context: str) -> List[ast.Stmt]:
        while self._match(TokenType.NEWLINE, TokenType.COMMENT, TokenType.ERROR):
            pass
        if not self._match(TokenType.INDENT):
            self._error(f"Expected an indented block after {context}.", self._peek().span)
            return []

        statements: List[ast.Stmt] = []
        while not self._is_at_end() and not self._check(TokenType.DEDENT):
            while self._match(TokenType.NEWLINE, TokenType.COMMENT, TokenType.DEDENT, TokenType.ERROR):
                if self._check(TokenType.DEDENT):
                    break
            if self._is_at_end() or self._check(TokenType.DEDENT):
                break
            stmt = self._statement()
            if stmt is not None:
                statements.append(stmt)

        while self._match(TokenType.NEWLINE, TokenType.COMMENT, TokenType.ERROR):
            pass
        if not self._match(TokenType.DEDENT):
            self._error(f"Expected the {context} block to end.", self._peek().span)
        return statements

    # Expressions -------------------------------------------------------------

    def _expression(self) -> ast.Expr:
        return self._or()

    def _or(self) -> ast.Expr:
        expr = self._and()
        while self._match(TokenType.OR):
            op = self._previous()
            right = self._and()
            span = self._span_from(expr.span, op)
            expr = ast.BinaryExpr(span=span, left=expr, operator="or", right=right)
        return expr

    def _and(self) -> ast.Expr:
        expr = self._not()
        while self._match(TokenType.AND):
            op = self._previous()
            right = self._not()
            span = self._span_from(expr.span, op)
            expr = ast.BinaryExpr(span=span, left=expr, operator="and", right=right)
        return expr

    def _not(self) -> ast.Expr:
        if self._match(TokenType.NOT):
            op = self._previous()
            operand = self._not()
            span = self._span_from(op, operand.span)
            return ast.UnaryExpr(span=span, operator="not", operand=operand)
        return self._comparison()

    def _comparison(self) -> ast.Expr:
        expr = self._additive()
        while self._match(TokenType.EQUAL_EQUAL, TokenType.BANG_EQUAL, TokenType.LESS, TokenType.GREATER, TokenType.LESS_EQUAL, TokenType.GREATER_EQUAL):
            op = self._previous()
            right = self._additive()
            span = self._span_from(expr.span, right.span)
            expr = ast.BinaryExpr(span=span, left=expr, operator=op.lexeme, right=right)
        return expr

    def _additive(self) -> ast.Expr:
        expr = self._multiplicative()
        while self._match(TokenType.PLUS, TokenType.MINUS):
            op = self._previous()
            right = self._multiplicative()
            span = self._span_from(expr.span, right.span)
            expr = ast.BinaryExpr(span=span, left=expr, operator=op.lexeme, right=right)
        return expr

    def _multiplicative(self) -> ast.Expr:
        expr = self._power()
        while self._match(TokenType.STAR, TokenType.SLASH, TokenType.PERCENT):
            op = self._previous()
            right = self._power()
            span = self._span_from(expr.span, right.span)
            expr = ast.BinaryExpr(span=span, left=expr, operator=op.lexeme, right=right)
        return expr

    def _power(self) -> ast.Expr:
        expr = self._unary()
        if self._match(TokenType.POWER):
            op = self._previous()
            right = self._power()
            span = self._span_from(expr.span, right.span)
            expr = ast.BinaryExpr(span=span, left=expr, operator="**", right=right)
        return expr

    def _unary(self) -> ast.Expr:
        if self._match(TokenType.MINUS):
            op = self._previous()
            operand = self._unary()
            span = self._span_from(op, operand.span)
            return ast.UnaryExpr(span=span, operator="-", operand=operand)
        return self._call()

    def _call(self) -> ast.Expr:
        expr = self._primary()
        while True:
            if self._check(TokenType.IDENTIFIER):
                name_tok = self._advance()
                span = self._span_from(expr.span, name_tok)
                expr = ast.PropertyExpr(span=span, object=expr, name=name_tok.value)
                continue
            if self._match(TokenType.WITH):
                args: List[ast.Expr] = []
                if not self._check(TokenType.DOT) and not self._is_at_end():
                    args.append(self._expression())
                    while self._match(TokenType.COMMA):
                        args.append(self._expression())
                span = self._span_from(expr.span, self._previous())
                expr = ast.CallExpr(span=span, callee=expr, arguments=args)
                continue
            break
        # Index access follows a call or primary.
        while self._match(TokenType.LBRACKET):
            index = self._expression()
            end = self._consume(TokenType.RBRACKET, "Expected ']' after index.")
            span = self._span_from(expr.span, end if end else self._previous())
            expr = ast.IndexExpr(span=span, object=expr, index=index)
        return expr

    def _primary(self) -> ast.Expr:
        if self._match(TokenType.TRUE):
            return ast.BooleanLiteral(span=self._previous().span, value=True)
        if self._match(TokenType.FALSE):
            return ast.BooleanLiteral(span=self._previous().span, value=False)
        if self._match(TokenType.NOTHING):
            return ast.NothingLiteral(span=self._previous().span)
        if self._match(TokenType.INPUT):
            return ast.InputExpr(span=self._previous().span)
        if self._match(TokenType.THIS):
            return ast.VariableExpr(span=self._previous().span, name="this")
        if self._match(TokenType.NEW):
            return self._new_expression()
        if self._match(TokenType.TELL):
            return self._tell_expression()
        if self._match(TokenType.THE):
            return self._the_expression()
        if self._match(TokenType.NUMBER):
            tok = self._previous()
            return ast.NumberLiteral(span=tok.span, value=tok.value)
        if self._match(TokenType.STRING):
            tok = self._previous()
            return ast.StringLiteral(span=tok.span, value=tok.value)
        if self._match(TokenType.IDENTIFIER):
            tok = self._previous()
            return ast.VariableExpr(span=tok.span, name=tok.value)
        if self._match(TokenType.LPAREN):
            expr = self._expression()
            self._consume(TokenType.RPAREN, "Expected ')' after expression.")
            return expr
        if self._match(TokenType.LBRACKET):
            return self._list_literal()
        if self._match(TokenType.LBRACE):
            return self._dict_literal()

        self._error(
            f"Unexpected token '{self._peek().lexeme}'. Expected an expression.",
            self._peek().span,
        )

    def _new_expression(self) -> ast.Expr:
        start = self._previous()
        class_expr = self._primary()
        arguments: List[ast.Expr] = []
        if self._match(TokenType.WITH):
            if not self._check(TokenType.DOT) and not self._is_at_end():
                arguments.append(self._expression())
                while self._match(TokenType.COMMA):
                    arguments.append(self._expression())
        span = self._span_from(start, self._previous())
        return ast.NewExpr(span=span, class_expr=class_expr, arguments=arguments)

    def _tell_expression(self) -> ast.Expr:
        start = self._previous()
        object_expr = self._expression()
        self._consume(TokenType.TO, "Expected 'to' after the object in a 'tell' expression.")
        method_tok = self._consume(TokenType.IDENTIFIER, "Expected a method name after 'to'.")
        arguments: List[ast.Expr] = []
        if self._match(TokenType.WITH):
            if not self._check(TokenType.DOT) and not self._is_at_end():
                arguments.append(self._expression())
                while self._match(TokenType.COMMA):
                    arguments.append(self._expression())
        end = self._previous()
        if method_tok is not None:
            end = method_tok
        span = self._span_from(start, end)
        return ast.TellExpr(span=span, object=object_expr, method=method_tok.value if method_tok else "", arguments=arguments)

    def _the_expression(self) -> ast.Expr:
        start = self._previous()
        name_tok = self._consume(TokenType.IDENTIFIER, "Expected a property name after 'the'.")
        self._consume(TokenType.OF, "Expected 'of' after the property name.")
        object_expr = self._expression()
        span = self._span_from(start, object_expr.span)
        return ast.PropertyExpr(
            span=span,
            object=object_expr,
            name=name_tok.value if name_tok else "",
        )

    def _list_literal(self) -> ast.Expr:
        start = self._previous()
        elements: List[ast.Expr] = []
        if not self._check(TokenType.RBRACKET):
            elements.append(self._expression())
            while self._match(TokenType.COMMA):
                elements.append(self._expression())
        end = self._consume(TokenType.RBRACKET, "Expected ']' after list elements.")
        if end is None:
            end = self._previous()
        span = self._span_from(start, end)
        return ast.ListExpr(span=span, elements=elements)

    def _dict_literal(self) -> ast.Expr:
        start = self._previous()
        pairs: List[tuple] = []
        if not self._check(TokenType.RBRACE):
            key = self._expression()
            self._consume(TokenType.COLON, "Expected ':' after dictionary key.")
            value = self._expression()
            pairs.append((key, value))
            while self._match(TokenType.COMMA):
                key = self._expression()
                self._consume(TokenType.COLON, "Expected ':' after dictionary key.")
                value = self._expression()
                pairs.append((key, value))
        end = self._consume(TokenType.RBRACE, "Expected '}' after dictionary entries.")
        if end is None:
            end = self._previous()
        span = self._span_from(start, end)
        return ast.DictExpr(span=span, pairs=pairs)

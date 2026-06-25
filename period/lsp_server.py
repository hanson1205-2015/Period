"""A minimal JSON-RPC LSP server for Period, implemented with only the stdlib."""
import json
import sys
from typing import Any, Dict, List, Optional

from . import ast_nodes as ast
from .lexer import Lexer, TokenType
from .parser import Parser
from .semantic import SemanticChecker


class Document:
    """Holds the current source text, cached AST, and symbol table for a file."""

    def __init__(self, uri: str, text: str):
        self.uri = uri
        self.text = text
        self.ast: Optional[ast.Program] = None
        self.diagnostics: List[dict] = []
        self.definitions: Dict[str, tuple] = {}  # name -> (line, col_start)
        self.top_level: Dict[str, dict] = {}  # name -> symbol info
        self.scoped: List[dict] = []  # variables/parameters with scope ranges
        self.methods: List[dict] = []  # class methods
        self.properties: List[dict] = []  # class properties (this.name assignments)
        self.imports: Dict[str, dict] = {}  # name -> imported symbol info
        self._func_returns: Dict[str, Optional[str]] = {}
        self._class_names: set = set()
        self._parse()

    def update(self, text: str):
        self.text = text
        self._parse()

    def _parse(self):
        lexer = Lexer(self.text, self.uri)
        tokens = lexer.scan()
        diagnostics = list(lexer.diagnostics)

        parser = Parser(tokens, self.text, self.uri)
        self.ast = parser.parse()
        diagnostics.extend(parser.diagnostics)

        if self.ast is not None and not diagnostics:
            semantic = SemanticChecker()
            for d in semantic.check(self.ast, self.uri):
                diagnostics.append(d)

        self.diagnostics = [d.to_dict() for d in diagnostics]

        self.definitions = {}
        self.top_level = {}
        self.scoped = []
        self.methods = []
        self.properties = []
        self.imports = {}
        if self.ast is not None:
            self._precompute_types()
            self._build_symbols()

    def _precompute_types(self):
        """Gather function return types and class names for type inference."""
        self._func_returns = {}
        self._class_names = set()
        if self.ast is None:
            return
        for stmt in self.ast.statements:
            if isinstance(stmt, ast.DefineStmt):
                self._func_returns[stmt.name] = stmt.return_type
            elif isinstance(stmt, ast.ClassStmt):
                self._class_names.add(stmt.name)
                for member in stmt.body:
                    if isinstance(member, ast.DefineStmt):
                        self._func_returns[member.name] = member.return_type

    @staticmethod
    def _block_end(statements: List[ast.Stmt]) -> Optional[int]:
        """Return the line number of the last statement in a block, if any."""
        if not statements:
            return None
        return statements[-1].span.line

    def _infer_type(self, expr: ast.Expr) -> str:
        """Infer a Period type name from an expression using available annotations."""
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
        if isinstance(expr, ast.CallExpr):
            if isinstance(expr.callee, ast.VariableExpr):
                name = expr.callee.name
                if name in self._func_returns and self._func_returns[name]:
                    return self._func_returns[name]
                if name == "length":
                    return "number"
                if name in {"string", "input"}:
                    return "string"
                if name == "number":
                    return "number"
                if name == "type":
                    return "string"
            return "unknown"
        if isinstance(expr, ast.VariableExpr):
            if expr.name in self._class_names:
                return expr.name
            return "unknown"
        return "unknown"

    def _find_properties(
        self,
        statements: List[ast.Stmt],
        local_types: Optional[Dict[str, str]] = None,
    ) -> Dict[str, str]:
        """Scan statements for assignments to this.<name> and infer their types."""
        if local_types is None:
            local_types = {}
        props: Dict[str, str] = {}
        for stmt in statements:
            if isinstance(stmt, ast.SetStmt):
                if (
                    isinstance(stmt.target, ast.PropertyExpr)
                    and isinstance(stmt.target.object, ast.VariableExpr)
                    and stmt.target.object.name == "this"
                ):
                    value_type = self._infer_type(stmt.value)
                    if value_type == "unknown" and isinstance(stmt.value, ast.VariableExpr):
                        value_type = local_types.get(stmt.value.name, "unknown")
                    props[stmt.target.name] = value_type
            elif isinstance(stmt, ast.IfStmt):
                props.update(self._find_properties(stmt.then_branch, local_types))
                props.update(self._find_properties(stmt.else_branch, local_types))
            elif isinstance(stmt, ast.WhileStmt):
                props.update(self._find_properties(stmt.body, local_types))
            elif isinstance(stmt, ast.BlockStmt):
                props.update(self._find_properties(stmt.statements, local_types))
        return props

    def _func_signature(
        self,
        name: str,
        parameters: List[str],
        parameter_types: List[Optional[str]],
        return_type: Optional[str],
        class_name: Optional[str] = None,
    ) -> str:
        parts = []
        for param, param_type in zip(parameters, parameter_types):
            if param_type:
                parts.append(f"{param}: {param_type}")
            else:
                parts.append(param)
        sig = f"{name}({', '.join(parts)})"
        if return_type:
            sig += f" -> {return_type}"
        if class_name:
            sig = f"{class_name}.{sig}"
        return sig

    def _record_symbol(
        self,
        name: str,
        kind: str,
        scope_start: int,
        scope_end: int,
        span,
        type_name: Optional[str] = None,
        signature: Optional[str] = None,
        docstring: Optional[str] = None,
        class_name: Optional[str] = None,
    ) -> dict:
        return {
            "name": name,
            "kind": kind,
            "scope_start": scope_start,
            "scope_end": scope_end,
            "line": span.line,
            "col_start": span.col_start,
            "col_end": span.col_end,
            "type_name": type_name,
            "signature": signature,
            "docstring": docstring,
            "class_name": class_name,
        }

    def _build_symbols(self):
        """Index user-defined symbols for hover, completion, and definition."""
        if self.ast is None:
            return
        global_end = 10**9
        self._collect_stmts(self.ast.statements, global_end)

    def _collect_stmts(self, statements: List[ast.Stmt], scope_end: int):
        for stmt in statements:
            if isinstance(stmt, ast.LetStmt):
                type_name = stmt.type_annotation or self._infer_type(stmt.initializer)
                sym = self._record_symbol(
                    stmt.name,
                    "variable",
                    stmt.span.line,
                    scope_end,
                    stmt.span,
                    type_name=type_name,
                )
                self.scoped.append(sym)
                self.definitions[sym["name"]] = (sym["line"], sym["col_start"])
            elif isinstance(stmt, ast.DefineStmt):
                body_end = self._block_end(stmt.body) or stmt.span.line
                sig = self._func_signature(
                    stmt.name,
                    stmt.parameters,
                    stmt.parameter_types,
                    stmt.return_type,
                )
                sym = self._record_symbol(
                    stmt.name,
                    "function",
                    1,
                    scope_end,
                    stmt.name_span,
                    signature=sig,
                    docstring=stmt.docstring,
                )
                self.top_level[sym["name"]] = sym
                self.definitions[sym["name"]] = (sym["line"], sym["col_start"])
                for param, param_type in zip(stmt.parameters, stmt.parameter_types):
                    ptype = param_type or "unknown"
                    self.scoped.append(
                        self._record_symbol(
                            param,
                            "parameter",
                            stmt.span.line,
                            body_end,
                            stmt.span,
                            type_name=ptype,
                        )
                    )
                self._collect_stmts(stmt.body, body_end)
            elif isinstance(stmt, ast.ClassStmt):
                sym = self._record_symbol(
                    stmt.name,
                    "class",
                    1,
                    scope_end,
                    stmt.name_span,
                    docstring=stmt.docstring,
                )
                self.top_level[sym["name"]] = sym
                self.definitions[sym["name"]] = (sym["line"], sym["col_start"])
                class_body_end = self._block_end(stmt.body) or stmt.span.line
                init_signature = None
                for member in stmt.body:
                    if isinstance(member, ast.InitStmt):
                        body_end = self._block_end(member.body) or member.span.line
                        init_signature = self._func_signature(
                            "init",
                            member.parameters,
                            member.parameter_types,
                            None,
                            class_name=stmt.name,
                        )
                        for param, param_type in zip(member.parameters, member.parameter_types):
                            ptype = param_type or "unknown"
                            self.scoped.append(
                                self._record_symbol(
                                    param,
                                    "parameter",
                                    member.span.line,
                                    body_end,
                                    member.span,
                                    type_name=ptype,
                                )
                            )
                        self._collect_stmts(member.body, body_end)
                    elif isinstance(member, ast.DefineStmt):
                        body_end = self._block_end(member.body) or member.span.line
                        sig = self._func_signature(
                            member.name,
                            member.parameters,
                            member.parameter_types,
                            member.return_type,
                            class_name=stmt.name,
                        )
                        self.methods.append(
                            self._record_symbol(
                                member.name,
                                "method",
                                member.span.line,
                                class_body_end,
                                member.name_span,
                                signature=sig,
                                docstring=member.docstring,
                                class_name=stmt.name,
                            )
                        )
                        for param, param_type in zip(member.parameters, member.parameter_types):
                            ptype = param_type or "unknown"
                            self.scoped.append(
                                self._record_symbol(
                                    param,
                                    "parameter",
                                    member.span.line,
                                    body_end,
                                    member.span,
                                    type_name=ptype,
                                )
                            )
                        self._collect_stmts(member.body, body_end)
                if init_signature:
                    sym["init_signature"] = init_signature
                props = {}
                for member in stmt.body:
                    if isinstance(member, ast.InitStmt):
                        local_types = {
                            p: t or "unknown"
                            for p, t in zip(member.parameters, member.parameter_types)
                        }
                        props.update(self._find_properties(member.body, local_types))
                    elif isinstance(member, ast.DefineStmt):
                        local_types = {
                            p: t or "unknown"
                            for p, t in zip(member.parameters, member.parameter_types)
                        }
                        props.update(self._find_properties(member.body, local_types))
                for prop_name, prop_type in props.items():
                    self.properties.append(
                        self._record_symbol(
                            prop_name,
                            "property",
                            stmt.span.line,
                            class_body_end,
                            stmt.name_span,
                            type_name=prop_type,
                            class_name=stmt.name,
                        )
                    )
            elif isinstance(stmt, ast.ImportStmt):
                self._collect_import_symbols(stmt)
            elif isinstance(stmt, ast.IfStmt):
                then_end = self._block_end(stmt.then_branch) or stmt.span.line
                self._collect_stmts(stmt.then_branch, then_end)
                else_end = self._block_end(stmt.else_branch) or stmt.span.line
                self._collect_stmts(stmt.else_branch, else_end)
            elif isinstance(stmt, ast.WhileStmt):
                body_end = self._block_end(stmt.body) or stmt.span.line
                self._collect_stmts(stmt.body, body_end)
            elif isinstance(stmt, ast.BlockStmt):
                block_end = self._block_end(stmt.statements) or stmt.span.line
                self._collect_stmts(stmt.statements, block_end)

    def _collect_import_symbols(self, stmt: ast.ImportStmt):
        """Index names exported by an imported module for completion/hover."""
        from .module_loader import resolve_module

        resolved = resolve_module(stmt.module_path, self.uri)
        if resolved is None:
            return

        if isinstance(resolved, str):
            # Built-in module.
            import importlib

            try:
                mod = importlib.import_module(f"period.stdlib.{resolved}")
            except Exception:
                return
            exports = getattr(mod, "EXPORTS", [])
            docs = getattr(mod, "DOCS", {})
            for name in exports:
                if not hasattr(mod, name):
                    continue
                value = getattr(mod, name)
                kind = "function" if callable(value) else "variable"
                signature = None
                docstring = None
                type_name = None
                doc_entry = docs.get(name)
                if isinstance(doc_entry, tuple):
                    signature, docstring = doc_entry
                elif isinstance(doc_entry, str):
                    docstring = doc_entry
                if not callable(value):
                    if isinstance(value, bool):
                        type_name = "boolean"
                    elif isinstance(value, (int, float)):
                        type_name = "number"
                    elif isinstance(value, str):
                        type_name = "string"
                    elif isinstance(value, list):
                        type_name = "list"
                    elif isinstance(value, dict):
                        type_name = "dictionary"
                    else:
                        type_name = type(value).__name__
                self.imports[name] = {
                    "name": name,
                    "kind": kind,
                    "line": stmt.module_span.line,
                    "col_start": stmt.module_span.col_start,
                    "col_end": stmt.module_span.col_end,
                    "detail": f"Imported from {resolved}",
                    "signature": signature,
                    "docstring": docstring,
                    "type_name": type_name,
                }
            return

        # File-based module.
        from .lexer import Lexer
        from .parser import Parser

        source = resolved.read_text(encoding="utf-8")
        lexer = Lexer(source, str(resolved))
        tokens = lexer.scan()
        parser = Parser(tokens, source, str(resolved))
        program = parser.parse()
        if parser.diagnostics:
            return
        module_name = resolved.name
        for s in program.statements:
            if isinstance(s, ast.DefineStmt):
                sig = self._func_signature(
                    s.name,
                    s.parameters,
                    s.parameter_types,
                    s.return_type,
                )
                self.imports[s.name] = {
                    "name": s.name,
                    "kind": "function",
                    "line": s.name_span.line,
                    "col_start": s.name_span.col_start,
                    "col_end": s.name_span.col_end,
                    "detail": f"Imported from {module_name}",
                    "signature": sig,
                    "docstring": s.docstring,
                }
            elif isinstance(s, ast.ClassStmt):
                self.imports[s.name] = {
                    "name": s.name,
                    "kind": "class",
                    "line": s.name_span.line,
                    "col_start": s.name_span.col_start,
                    "col_end": s.name_span.col_end,
                    "detail": f"Imported from {module_name}",
                    "docstring": s.docstring,
                }
            elif isinstance(s, ast.LetStmt):
                type_name = s.type_annotation or self._infer_type(s.initializer)
                self.imports[s.name] = {
                    "name": s.name,
                    "kind": "variable",
                    "line": s.span.line,
                    "col_start": s.span.col_start,
                    "col_end": s.span.col_end,
                    "detail": f"Imported from {module_name}",
                    "type_name": type_name,
                }


class LSPServer:
    """Very small LSP server speaking JSON-RPC over stdin/stdout."""

    def __init__(self, in_stream=sys.stdin.buffer, out_stream=sys.stdout.buffer):
        self.in_stream = in_stream
        self.out_stream = out_stream
        self.documents: Dict[str, Document] = {}
        self.running = True
        self.shutdown_requested = False

    def run(self):
        while self.running:
            message = self._read_message()
            if message is None:
                continue
            self._handle_message(message)

    def _read_message(self) -> Optional[dict]:
        headers = {}
        while True:
            line = self.in_stream.readline()
            if not line:
                self.running = False
                return None
            line = line.decode("utf-8").strip()
            if line == "":
                break
            key, value = line.split(":", 1)
            headers[key.strip().lower()] = value.strip()
        length = int(headers.get("content-length", "0"))
        if length == 0:
            return None
        body = self.in_stream.read(length)
        return json.loads(body.decode("utf-8"))

    def _send(self, payload: dict):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        header = f"Content-Length: {len(body)}\r\n\r\n".encode("ascii")
        self.out_stream.write(header + body)
        self.out_stream.flush()

    def _respond(self, request_id: Any, result: Any):
        self._send({"jsonrpc": "2.0", "id": request_id, "result": result})

    def _notify(self, method: str, params: Any):
        self._send({"jsonrpc": "2.0", "method": method, "params": params})

    def _handle_message(self, message: dict):
        method = message.get("method")
        request_id = message.get("id")
        params = message.get("params", {})

        if method == "initialize":
            self._respond(request_id, self._initialize(params))
        elif method == "initialized":
            pass
        elif method == "shutdown":
            self.shutdown_requested = True
            self._respond(request_id, None)
        elif method == "exit":
            self.running = False
        elif method == "textDocument/didOpen":
            self._did_open(params)
        elif method == "textDocument/didChange":
            self._did_change(params)
        elif method == "textDocument/didSave":
            self._did_save(params)
        elif method == "textDocument/didClose":
            self._did_close(params)
        elif method == "textDocument/hover":
            self._hover(request_id, params)
        elif method == "textDocument/completion":
            self._completion(request_id, params)
        elif method == "textDocument/formatting":
            self._formatting(request_id, params)
        elif method == "textDocument/definition":
            self._definition(request_id, params)
        elif method == "textDocument/semanticTokens/full":
            self._semantic_tokens_full(request_id, params)
        else:
            if request_id is not None:
                self._respond(request_id, None)

    def _initialize(self, params: dict) -> dict:
        return {
            "capabilities": {
                "textDocumentSync": {
                    "openClose": True,
                    "change": 1,  # Full document sync.
                    "save": True,
                },
                "hoverProvider": True,
                "completionProvider": {"triggerCharacters": ["."]},
                "documentFormattingProvider": True,
                "definitionProvider": True,
                "semanticTokensProvider": {
                    "legend": {
                        "tokenTypes": [
                            "function",
                            "class",
                            "variable",
                            "parameter",
                            "property",
                            "method",
                        ],
                        "tokenModifiers": ["declaration", "readonly", "defaultLibrary"],
                    },
                    "full": {"delta": False},
                },
            },
            "serverInfo": {"name": "period-lsp", "version": "0.0.1"},
        }

    def _did_open(self, params: dict):
        text_doc = params["textDocument"]
        doc = Document(text_doc["uri"], text_doc["text"])
        self.documents[text_doc["uri"]] = doc
        self._publish_diagnostics(text_doc["uri"], doc.diagnostics)

    def _did_change(self, params: dict):
        text_doc = params["textDocument"]
        content = params["contentChanges"][0]["text"]
        doc = self.documents.get(text_doc["uri"])
        if doc is None:
            doc = Document(text_doc["uri"], content)
            self.documents[text_doc["uri"]] = doc
        else:
            doc.update(content)
        self._publish_diagnostics(text_doc["uri"], doc.diagnostics)

    def _did_save(self, params: dict):
        pass

    def _did_close(self, params: dict):
        self.documents.pop(params["textDocument"]["uri"], None)

    def _publish_diagnostics(self, uri: str, diagnostics: List[dict]):
        items = [
            {
                "range": {
                    "start": {"line": d["line"] - 1, "character": d["col_start"] - 1},
                    "end": {"line": d["line"] - 1, "character": d["col_end"] - 1},
                },
                "severity": 1 if d["severity"] == "error" else 2,
                "message": d["message"],
                "source": "period",
            }
            for d in diagnostics
        ]
        self._notify("textDocument/publishDiagnostics", {"uri": uri, "diagnostics": items})

    def _hover(self, request_id: Any, params: dict):
        uri = params["textDocument"]["uri"]
        pos = params["position"]
        doc = self.documents.get(uri)
        if doc is None:
            self._respond(request_id, None)
            return
        word = self._word_at(doc.text, pos["line"], pos["character"])
        hover_text = self._hover_text(word)
        symbol_text = self._hover_symbol_text(doc, word, pos["line"] + 1, pos["character"] + 1)
        if symbol_text:
            hover_text = f"{symbol_text}\n\n---\n\n{hover_text}" if hover_text else symbol_text
        if hover_text:
            self._respond(
                request_id,
                {
                    "contents": {
                        "kind": "markdown",
                        "value": hover_text,
                    }
                },
            )
        else:
            self._respond(request_id, None)

    def _lookup_symbols(self, doc: Document, word: str, line: int, character: int) -> List[dict]:
        """Return symbol records that match the word at the given 1-based position."""
        # Prefer scoped symbols (variables/parameters) that are currently in scope.
        scoped = [
            s
            for s in doc.scoped
            if s["name"] == word and s["scope_start"] <= line <= s["scope_end"]
        ]
        if scoped:
            scoped.sort(key=lambda s: s["scope_start"], reverse=True)
            return scoped
        if word in doc.top_level:
            return [doc.top_level[word]]
        methods = [s for s in doc.methods if s["name"] == word]
        if methods:
            return methods
        properties = [s for s in doc.properties if s["name"] == word]
        if properties:
            return properties
        imported = doc.imports.get(word)
        if imported:
            return [imported]
        return []

    def _hover_symbol_text(self, doc: Document, word: str, line: int, character: int) -> Optional[str]:
        symbols = self._lookup_symbols(doc, word, line, character)
        if not symbols:
            return None
        parts = []
        for symbol in symbols:
            text = self._format_hover(symbol)
            if text:
                parts.append(text)
        return "\n\n---\n\n".join(parts) if parts else None

    def _format_hover(self, symbol: dict) -> Optional[str]:
        kind = symbol["kind"]
        name = symbol["name"]
        docstring = symbol.get("docstring")
        lines = []
        if kind in ("function", "method"):
            lines.append(f"```period")
            lines.append(symbol.get("signature") or name)
            lines.append("```")
            if docstring:
                lines.append(docstring)
        elif kind == "class":
            lines.append(f"```period")
            lines.append(f"class {name}")
            init_sig = symbol.get("init_signature")
            if init_sig:
                lines.append(init_sig)
            lines.append("```")
            if docstring:
                lines.append(docstring)
        elif kind in ("variable", "parameter", "property"):
            lines.append(f"```period")
            type_name = symbol.get("type_name") or "unknown"
            lines.append(f"{name}: {type_name}")
            lines.append("```")
        else:
            return None
        if symbol.get("detail"):
            lines.append(f"*{symbol['detail']}*")
        return "\n".join(lines)

    def _word_at(self, text: str, line: int, character: int) -> str:
        lines = text.splitlines()
        if line >= len(lines):
            return ""
        line_text = lines[line]
        if character >= len(line_text):
            return ""
        start = character
        while start > 0 and (line_text[start - 1].isalnum() or line_text[start - 1] == "_"):
            start -= 1
        end = character
        while end < len(line_text) and (line_text[end].isalnum() or line_text[end] == "_"):
            end += 1
        return line_text[start:end]

    def _hover_text(self, word: str) -> Optional[str]:
        docs = {
            "let": "`let <name> be <value>.`\n\nDeclare a new variable.",
            "set": "`set <target> to <value>.`\n\nAssign a new value to an existing variable or property.",
            "if": "`if <condition> then.`\n`    <statements>`\n`[otherwise.`\n`    <statements>]`\n\nConditional statement using indentation.",
            "while": "`while <condition> repeat.`\n`    <statements>`\n\nLoop while the condition is true.",
            "define": "`define <name> [with <args>].`\n`    <statements>`\n\nDefine a function or method using indentation.",
            "return": "`return [value].`\n\nReturn a value from a function or method.",
            "class": "`class <Name>.`\n`    init [with <args>].`\n`        <statements>`\n`    define <method> [with <args>].`\n`        <statements>`\n\nDefine a class.",
            "init": "`init [with <args>].`\n`    <statements>`\n\nConstructor inside a class.",
            "this": "Refers to the current instance inside a class method or init.",
            "new": "`new <Class> [with <args>].`\n\nCreate a new instance of a class.",
            "tell": "`tell <object> to <method> [with <args>].`\n\nCall a method on an instance.",
            "the": "`the <property> of <object>.`\n\nRead a property of an instance.",
            "of": "Used with `the` to read an instance property.",
            "true": "Boolean true value.",
            "false": "Boolean false value.",
            "show": (
                "```period\n"
                "show <expression>\n"
                "```\n\n"
                "Print the value of the expression to standard output.\n\n"
                "Example: `show 1 + 2.` prints `3`."
            ),
            "input": (
                "```period\n"
                "input -> string\n"
                "```\n\n"
                "Read a line from standard input and return it as a string.\n\n"
                "Example: `let name be input.`"
            ),
            "length": (
                "```period\n"
                "length with <value> -> number\n"
                "```\n\n"
                "Return the length of a string, list, or dictionary.\n\n"
                "Examples:\n"
                "- `length with \"hello\"` returns `5`.\n"
                "- `length with [1, 2, 3]` returns `3`."
            ),
            "string": (
                "```period\n"
                "string with <value> -> string\n"
                "```\n\n"
                "Convert a value to a string.\n\n"
                "Also used as the `string` type annotation.\n\n"
                "Example: `string with 42.` returns `\"42\"`."
            ),
            "number": (
                "```period\n"
                "number with <value> -> number\n"
                "```\n\n"
                "Convert a value to a number. Booleans become `0` or `1`.\n\n"
                "Also used as the `number` type annotation, which accepts both integers and floating-point values.\n\n"
                "Example: `number with \"3.14\".` returns `3.14`."
            ),
            "type": (
                "```period\n"
                "type with <value> -> string\n"
                "```\n\n"
                "Return the name of the value's type as a string.\n\n"
                "Examples:\n"
                "- `type with 5.` returns `\"integer\"`.\n"
                "- `type with [1, 2].` returns `\"list\"`."
            ),
            "any": (
                "```period\n"
                "any\n"
                "```\n\n"
                "The `any` type annotation matches any value. It is useful when a variable may hold values of different types."
            ),
            "never": (
                "```period\n"
                "never\n"
                "```\n\n"
                "The `never` type annotation never matches a value. It represents an impossible type."
            ),
            "nothing": (
                "```period\n"
                "nothing\n"
                "```\n\n"
                "The absence of a value. `nothing` is the value returned by functions that do not explicitly return a value.\n\n"
                "Also used as the `nothing` type annotation."
            ),
            "boolean": (
                "```period\n"
                "boolean\n"
                "```\n\n"
                "The `boolean` type annotation matches the values `true` and `false`."
            ),
            "integer": (
                "```period\n"
                "integer\n"
                "```\n\n"
                "The `integer` type annotation matches whole numbers without a fractional part.\n\n"
                "Unlike `number`, an `integer` annotation rejects floating-point values."
            ),
            "list": (
                "```period\n"
                "list\n"
                "```\n\n"
                "The `list` type annotation matches list values created with `[...]`."
            ),
            "dictionary": (
                "```period\n"
                "dictionary\n"
                "```\n\n"
                "The `dictionary` type annotation matches dictionary values created with `{key: value, ...}`."
            ),
            "function": (
                "```period\n"
                "function\n"
                "```\n\n"
                "The `function` type annotation matches user-defined functions and built-in functions."
            ),
        }
        return docs.get(word)

    def _completion(self, request_id: Any, params: dict):
        items = []
        keywords = [
            ("let", "Declare a variable"),
            ("be", "Used in let statements"),
            ("set", "Assign a value"),
            ("to", "Used in set statements"),
            ("show", "Print a value"),
            ("if", "Start a conditional"),
            ("then", "Used after if condition"),
            ("otherwise", "Else branch"),
            ("while", "Start a loop"),
            ("repeat", "Used after while condition"),
            ("define", "Define a function or method"),
            ("with", "Used in calls/signatures"),
            ("return", "Return from function"),
            ("and", "Logical and"),
            ("or", "Logical or"),
            ("not", "Logical not"),
            ("true", "Boolean true"),
            ("false", "Boolean false"),
            ("nothing", "Null value"),
            ("input", "Read input"),
            ("class", "Define a class"),
            ("init", "Class constructor"),
            ("this", "Current instance"),
            ("new", "Create an instance"),
            ("tell", "Call a method"),
            ("of", "Property access"),
            ("the", "Property access"),
            ("import", "Import a module"),
            ("length", "Built-in: length"),
            ("string", "Built-in: string"),
            ("number", "Built-in: number"),
            ("type", "Built-in: type"),
        ]
        for kw, detail in keywords:
            items.append(
                {
                    "label": kw,
                    "kind": 14,  # Keyword
                    "detail": detail,
                    "insertText": kw,
                }
            )

        kind_map = {
            "function": 3,
            "method": 2,
            "class": 7,
            "variable": 6,
            "parameter": 6,
            "property": 5,
        }

        uri = params["textDocument"]["uri"]
        doc = self.documents.get(uri)
        if doc and doc.ast:
            for name in doc.definitions:
                items.append(
                    {
                        "label": name,
                        "kind": 6,  # Variable
                        "detail": "Defined in this file",
                        "insertText": name,
                    }
                )
            for info in doc.imports.values():
                items.append(
                    {
                        "label": info["name"],
                        "kind": kind_map.get(info["kind"], 6),
                        "detail": info.get("detail", "Imported"),
                        "insertText": info["name"],
                    }
                )

        self._respond(request_id, {"items": items, "isIncomplete": False})

    def _formatting(self, request_id: Any, params: dict):
        uri = params["textDocument"]["uri"]
        doc = self.documents.get(uri)
        if doc is None:
            self._respond(request_id, None)
            return
        formatted = self._format_source(doc.text)
        self._respond(
            request_id,
            [
                {
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {
                            "line": len(doc.text.splitlines()),
                            "character": 0,
                        },
                    },
                    "newText": formatted,
                }
            ],
        )

    def _format_source(self, text: str) -> str:
        lexer = Lexer(text, "<format>")
        tokens = lexer.scan()

        line_indent: Dict[int, int] = {}
        level = 0
        for tok in tokens:
            if tok.type == TokenType.INDENT:
                level += 1
            elif tok.type == TokenType.DEDENT:
                level -= 1
            elif tok.type in (TokenType.NEWLINE, TokenType.EOF):
                pass
            else:
                line = tok.span.line
                if line not in line_indent:
                    line_indent[line] = level

        lines = text.splitlines()
        out_lines: List[str] = []
        for idx, raw in enumerate(lines, start=1):
            stripped = raw.strip()
            if stripped == "":
                out_lines.append("")
                continue
            target = line_indent.get(idx, 0)
            out_lines.append("    " * target + stripped)
        return "\n".join(out_lines)

    def _definition(self, request_id: Any, params: dict):
        uri = params["textDocument"]["uri"]
        pos = params["position"]
        doc = self.documents.get(uri)
        if doc is None:
            self._respond(request_id, None)
            return
        word = self._word_at(doc.text, pos["line"], pos["character"])
        loc = doc.definitions.get(word)
        if loc is None:
            self._respond(request_id, None)
            return
        line, col = loc
        self._respond(
            request_id,
            {
                "uri": uri,
                "range": {
                    "start": {"line": line - 1, "character": col - 1},
                    "end": {"line": line - 1, "character": col - 1 + len(word)},
                },
            },
        )


    def _semantic_tokens_full(self, request_id: Any, params: dict):
        uri = params["textDocument"]["uri"]
        doc = self.documents.get(uri)
        if doc is None or doc.ast is None:
            self._respond(request_id, {"data": []})
            return

        checker = SemanticChecker()
        tokens = checker.semantic_tokens(doc.ast, doc.uri)
        self._respond(request_id, {"data": self._encode_semantic_tokens(tokens)})

    def _encode_semantic_tokens(self, tokens) -> List[int]:
        """Encode semantic tokens as LSP uint32 array.

        Each token is 5 integers: deltaLine, deltaStartChar, length, tokenType, tokenModifiers.
        Tokens must be sorted by line and column.
        """
        legend_types = [
            "function",
            "class",
            "variable",
            "parameter",
            "property",
            "method",
        ]
        legend_modifiers = ["declaration", "readonly", "defaultLibrary"]

        kind_to_type = {
            "function": legend_types.index("function"),
            "class": legend_types.index("class"),
            "variable": legend_types.index("variable"),
            "parameter": legend_types.index("parameter"),
            "property": legend_types.index("property"),
            "method": legend_types.index("method"),
            "builtin": legend_types.index("function"),
        }

        # Sort tokens by position.
        sorted_tokens = sorted(tokens, key=lambda t: (t[0].line, t[0].col_start))

        data: List[int] = []
        prev_line = 0
        prev_col = 0
        for span, kind, is_declaration in sorted_tokens:
            if span.line <= 0:
                continue
            line = span.line - 1  # LSP uses 0-based lines.
            col_start = span.col_start - 1  # LSP uses 0-based columns.
            length = span.col_end - span.col_start
            if length <= 0:
                continue

            token_type = kind_to_type.get(kind, legend_types.index("variable"))
            modifiers = 0
            if is_declaration:
                modifiers |= 1 << legend_modifiers.index("declaration")
            if kind == "builtin":
                modifiers |= 1 << legend_modifiers.index("readonly")
                modifiers |= 1 << legend_modifiers.index("defaultLibrary")

            delta_line = line - prev_line
            delta_col = col_start - prev_col if delta_line == 0 else col_start

            data.extend([delta_line, delta_col, length, token_type, modifiers])
            prev_line = line
            prev_col = col_start

        return data


def main():
    server = LSPServer()
    server.run()

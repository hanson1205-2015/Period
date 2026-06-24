"""A minimal JSON-RPC LSP server for Period, implemented with only the stdlib."""
import json
import sys
from typing import Any, Dict, List, Optional

from . import ast_nodes as ast
from .lexer import Lexer, TokenType
from .parser import Parser


class Document:
    """Holds the current source text and cached AST for a file."""

    def __init__(self, uri: str, text: str):
        self.uri = uri
        self.text = text
        self.ast: Optional[ast.Program] = None
        self.diagnostics: List[dict] = []
        self.definitions: Dict[str, tuple] = {}  # name -> (line, col_start)
        self._parse()

    def update(self, text: str):
        self.text = text
        self._parse()

    def _parse(self):
        try:
            tokens = Lexer(self.text, self.uri).scan()
        except Exception as e:
            self.ast = ast.Program(span=None, statements=[])
            self.diagnostics = []
            return
        parser = Parser(tokens, self.text, self.uri)
        self.ast = parser.parse()
        self.diagnostics = [d.to_dict() for d in parser.diagnostics]
        self.definitions = {}
        for stmt in self.ast.statements:
            if isinstance(stmt, ast.LetStmt):
                self.definitions[stmt.name] = (stmt.span.line, stmt.span.col_start)
            elif isinstance(stmt, ast.DefineStmt):
                self.definitions[stmt.name] = (stmt.span.line, stmt.span.col_start)


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
            "set": "`set <target> to <value>.`\n\nAssign a new value to an existing variable or index.",
            "show": "`show <expression>.`\n\nPrint the value of the expression.",
            "if": "`if <condition> then. ... end if.`\n\nConditional statement.",
            "while": "`while <condition> repeat. ... end while.`\n\nLoop while the condition is true.",
            "define": "`define <name> [with <args>]. ... end define.`\n\nDefine a function.",
            "return": "`return [value].`\n\nReturn a value from a function.",
            "true": "Boolean true value.",
            "false": "Boolean false value.",
            "nothing": "The absence of a value.",
            "input": "`input` reads a line from standard input.",
            "length": "`length with <value>` returns the length of a string, list, or dictionary.",
            "string": "`string with <value>` converts a value to a string.",
            "number": "`number with <value>` converts a value to a number.",
            "type": "`type with <value>` returns the name of the value's type.",
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
            ("end", "End a block"),
            ("while", "Start a loop"),
            ("repeat", "Used after while condition"),
            ("define", "Define a function"),
            ("with", "Used in function calls/signatures"),
            ("return", "Return from function"),
            ("and", "Logical and"),
            ("or", "Logical or"),
            ("not", "Logical not"),
            ("true", "Boolean true"),
            ("false", "Boolean false"),
            ("nothing", "Null value"),
            ("input", "Read input"),
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
        lines = text.splitlines()
        out_lines = []
        indent = 0
        for raw in lines:
            stripped = raw.strip()
            if stripped == "":
                out_lines.append("")
                continue
            # Decrease indent for closing block markers.
            lower = stripped.lower()
            if lower.startswith("end ") or lower == "end" or lower.startswith("otherwise"):
                indent = max(0, indent - 1)
            out_lines.append("    " * indent + stripped)
            # Increase indent for opening block markers.
            if lower.startswith("if ") or lower.startswith("while ") or lower.startswith("define "):
                indent += 1
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


def main():
    server = LSPServer()
    server.run()

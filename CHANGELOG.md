# Changelog

## 1.0.1 (2026-06-27)

### What's new

- Added a Rust-based LSP server (`period --lsp`).
- Hover information for variables, functions, classes, modules, and built-ins.
- Auto-completion for local symbols, built-ins, and module exports.
- Simple type inference based on function return-type annotations and literal kinds.
- Docstrings are now preserved and shown in hover popups.
- Diagnostics for parse/lex errors and undefined names.
- Fixed LSP server startup when the VS Code: client passes extra stdio flags.
- Fixed lexer panic on Windows CRLF line endings.
- Numeric fast-path now falls back to the interpreter when `rustc` is not available.
- Fixed false-positive "undefined variable" diagnostics for variables defined earlier in the same block (e.g. inside `while`/`if` bodies).
- Improved hover: variable/function signature is shown as a syntax-highlighted `period` code block on the first line, variables defined inside blocks (e.g. inside `while`) also show hover, and keywords like `show` now have hover docs.
- Fixed hover token-length matching for multi-character keywords (`show`, `returns`, etc.).
- The installer now uninstalls the old VS Code extension before installing the new one, preventing version-downgrade issues.

### Full commit

`760a43e`

## 1.0.0 (2026-06-27)

### What's new

- First stable release of Period.
- Complete rewrite of the language implementation in Rust under the `period/` crate.
- Lexer, parser, interpreter, numeric fast-path compiler, CLI, and Windows installer are all Rust-based.
- Removed the previous Python implementation and build tooling.
- Numeric programs are automatically translated to Rust and compiled to native code.
- Full interpreter support for strings, lists, dictionaries, classes, functions, imports, and built-in modules.

### Full commit

`04e01e9`

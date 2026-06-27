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
- Restored `period/stdlib/` as a directory of loadable modules. `list` and `text` are implemented as `.period` source files; `math`, `random`, `string`, and `time` are native modules with `.periodi` stub files for documentation and IDE support.
- Added support for `.periodi` interface files: they are parsed by the LSP for completions/hover but ignored by the runtime, similar to Python `.pyi` stubs. Function bodies can be written as `...`.
- Fixed syntax gaps found in docs.html audit:
  - Keywords and reserved words must be lowercase; any capitalization (e.g. `Let` or `LET`) is a lexer error.
  - `true`/`false` are now boolean values and `nothing` is the nothing value, not numbers.
  - Zero-argument built-ins like `input` can be used without `with`.
  - `import` with a plain name resolves to built-in or standard-library modules only; local files must be imported with a relative path (`.helper`, `..helper`) or from a `lib/` folder.
  - Updated the grammar reference and module list in `docs/docs.html` to match the Rust implementation.
- Fixed an LSP server crash when lexing files containing invalid keyword casing; such errors are now reported as diagnostics instead of crashing the server.
- Updated VS Code: syntax highlighting so module names in `import` / `from` statements are colored green, and common functions exported by built-in/standard-library modules (e.g. `sin`, `upper`, `sum`) are colored yellow.
- The Windows installer now registers `.periodi` files as "Period Interface File" with the Period icon and open command.
- The VS Code: extension now associates `.periodi` files with the Period language and contributes a "Period Icons" file icon theme for `.period` / `.periodi` files.
- Zero-argument user-defined functions are now auto-called when used as values, matching zero-argument built-ins.
- A leading string literal inside a block is now treated as a docstring and does not require a trailing `.`, enabling stub/interface files to declare documentation before `...`.
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

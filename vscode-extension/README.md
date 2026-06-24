# Period Language Support for VS Code

This extension provides rich language support for **Period**, an elegant programming language that ends every statement with a period.

## Features

- **Syntax highlighting** for Period source files (`.period`).
- **LSP-powered diagnostics** with precise line and column error markers.
- **Hover information** for keywords and built-in functions.
- **Auto-completion** for keywords, built-ins, and locally defined names.
- **Document formatting** with automatic indentation for blocks.
- **Go to Definition** for variables and functions.

## Requirements

The extension launches the Period language server. Make sure `period` (or `period.exe` on Windows) is on your PATH, or configure `period.languageServerPath` in VS Code settings.

## Installation

Install the `.vsix` file from a release:

```bash
code --install-extension period-language.vsix
```

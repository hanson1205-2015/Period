# Period

An elegant English programming language where every statement ends with a period.

```period
-- Greet the world.
let greeting be "Hello, World!".
show greeting.
```

## Features

- Sentence-like syntax for readable code.
- Detailed error messages with exact line and column information.
- Parser recovers from errors to report multiple issues at once.
- Turing complete: variables, conditionals, loops, functions, classes, and recursion.
- Modules and standard library: import built-in modules (math, string, random, time, json, os) or local `.period` files.
- VS Code extension with syntax highlighting, hover, completion, formatting, go-to-definition, and LSP diagnostics.
- Command-line compiler and interactive REPL.

## Quick Start

Install with the Windows installer from the [releases page](https://github.com/period-lang/period/releases), then run:

```bash
period hello.period
```

Start the REPL:

```bash
period
```

## Building from Source

```bash
pip install -r requirements.txt
python build.py                 # Builds dist/period.exe with Nuitka
cd vscode-extension && npx @vscode/vsce package  # Builds the VSIX
```

## Documentation

The full documentation is included in the `docs/` folder as a self-contained static website. Open `docs/index.html` in a browser after installation.

## License

MIT

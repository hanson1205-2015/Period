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

The language is implemented entirely in Rust under `period-rs`.

```bash
cd period-rs
cargo build --release
```

This produces `target/release/period.exe`. Run a program with:

```bash
period hello.period
```

Numeric programs are automatically compiled to native code via a Rust fast path; richer programs fall back to the built-in interpreter.

## Documentation

The full documentation is included in the `docs/` folder as a self-contained static website. Open `docs/index.html` in a browser after installation.

## License

MIT

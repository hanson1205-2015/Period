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

The language is implemented in Rust under `period/`. On Windows the release build also produces a small C wrapper (`period.exe`) for extra-fast startup; on Linux and macOS you can use the Rust binary directly.

### Windows

```bash
cd period
cargo build --release
```

This produces `target/release/period.exe`. The full distribution (including the TCC JIT compiler) is built with:

```bash
python scripts/build_dist.py
```

### Linux / macOS

```bash
cd period
cargo build --release
```

The binary is at `target/release/period`. Copy or symlink it to your PATH:

```bash
sudo cp target/release/period /usr/local/bin/period
```

For the JIT numeric backend, install [TCC](https://bellard.org/tcc/) (`tcc`) and make sure it is on your PATH. Without TCC, numeric programs fall back to the interpreter.

Run a program with:

```bash
period hello.period
```

## Documentation

The full documentation is included in the `docs/` folder as a self-contained static website. Open `docs/index.html` in a browser after installation.

## License

MIT

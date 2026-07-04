# Period

An elegant English programming language where every statement ends with a period. It also accepts a familiar dot-and-parentheses compact syntax, so you can write `obj.method()` when `tell obj to method with ...` feels too verbose.

```period
-- Greet the world.
let greeting be "Hello, World!".
show greeting.
```

## Features

- Sentence-like syntax for readable code.
- Detailed error messages with exact line and column information.
- Turing complete: variables, conditionals, loops, functions, classes, and recursion.
- Modules and standard library: import built-in modules (math, string, random, time, list, text, path, test) or local `.period` files.
- VS Code extension with syntax highlighting, hover, completion, and LSP diagnostics.
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

```
Period REPL. Type 'exit.' or 'quit.' to leave, or Ctrl+C.

>>> let x be 10.
>>> show x * 2.
20
>>> exit.
```

## Building from Source

The language is implemented in Rust under `period/`. On Windows the release build also produces a small C wrapper (`period.exe`) for fast startup; on Linux and macOS you can use the Rust binary directly.

### Windows

```bash
cd period
cargo build --release
```

This produces `target/release/period.exe`. The full Windows distribution is built with:

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

Run a program with:

```bash
period hello.period
```

## Language Notes

- **Truthiness is strict.** Only booleans can be used as conditions; strings, numbers, lists, and dictionaries are not implicitly truthy or falsy. Use explicit comparisons such as `if the length of xs > 0 then:` or `if name != "" then:`.
- **Type annotations are optional.** Unannotated code is checked dynamically; where annotations are given, the static type checker validates them before execution.
- **Function call arguments are full expressions.** `f with a + b` is parsed as `f(a + b)`, and `add with x + 1, y + 1` is parsed as `add(x + 1, y + 1)`. Parentheses are only needed to group expressions differently or to disambiguate nested calls.

## Standard Library Modules

Built-in modules can be imported directly by name:

```period
import list.
show sum with [1, 2, 3, 4].

import text.
show join with ["Hello", "World"], " ".
```

Available source modules include `list` (sum, max, min, length helpers) and `text` (join and other string utilities). Built-in native modules include `math`, `string`, `random`, and `time`.

## Package Manager

Period includes a small built-in package manager for sharing modules.

```bash
# Start a new project
period init myproject

# Install a package from the registry
period install hello

# Install from a URL or local file
period install https://example.com/mypkg.period
period install ./mypkg.period

# Update all dependencies
period update
```

The default registry is hosted on GitHub at `https://raw.githubusercontent.com/ExploreMaths/Period/main/registry`. Set the `PERIOD_REGISTRY` environment variable to use a different registry.

### Publishing a package

Period uses a GitHub-based static registry. All packages are stored as files in the `registry/` directory of this repository, and changes are submitted through Pull Requests.

For project maintainers with write access (requires Git installed and in PATH):

```bash
period publish ./mypkg.period --name mypkg --version 1.0.0 --push
```

This generates the registry files and runs `git add`, `git commit`, and `git push` automatically.

For other contributors, use the standard fork-and-pull-request workflow:

1. Fork `ExploreMaths/Period` on GitHub.
2. Clone your fork and add the registry as a git remote if needed:
   ```bash
   git clone https://github.com/<your-username>/Period.git
   cd Period
   git remote add upstream https://github.com/ExploreMaths/Period.git
   ```
3. Publish and push to your own fork:
   ```bash
   period publish ./mypkg.period --name mypkg --version 1.0.0 --push --remote origin
   ```
4. Open a Pull Request on GitHub to merge your changes into `ExploreMaths/Period`.

Once the PR is merged, everyone can install the package with:

```bash
period install mypkg
```

## Documentation

The full documentation is included in the `docs/` folder as a self-contained static website. Open `docs/index.html` in a browser after installation.

## License

MIT

# Changelog

## 1.0.0 (2026-06-27)

### What is Period?

Period is an elegant programming language that reads like English — every statement ends with a period. It is designed to be readable, expressive, and fast enough for everyday scripting and numeric workloads.

### What’s new in 1.0.0

This is the first stable release of Period and marks a complete rewrite of the language implementation in Rust. The previous Python-based toolchain has been fully replaced, so the resulting executable runs without any external interpreter or runtime.

#### Rust-first implementation

- Lexer, parser, semantic handling, interpreter, numeric compiler, and CLI are all implemented in a single Rust crate under the `period/` directory.
- Build with Cargo:

```bash
cd period
cargo build --release
```

The produced binary is `target/release/period` (or `period.exe` on Windows).

#### Numeric fast path

For programs that use only numeric operations, loops, conditionals, and functions, Period automatically:

1. Translates the program to Rust.
2. Compiles it with `rustc -C opt-level=3 -C target-cpu=native`.
3. Caches the resulting executable in a temporary directory.
4. Runs the cached executable directly on subsequent runs.

This makes loop-heavy numeric code comparable in speed to hand-written Rust or C.

#### Full-featured interpreter

Programs that use strings, lists, dictionaries, classes, imports, or built-in modules fall back to a fast tree-walking interpreter included in the same binary. No separate runtime is needed.

### Language features

- **Readable syntax**

```period
let greeting be "Hello, World!".
show greeting.
```

- **Variables and arithmetic**

```period
let a be 10.
let b be a + 5.
show b.
```

- **Conditionals and loops**

```period
if a > 5 then:
    show "big".

while a > 0 repeat:
    set a to a - 1.

for i in range 10:
    show i.
```

- **Functions**

```period
define factorial with n:
    let result be 1.
    while n > 1 repeat:
        set result to result * n.
        set n to n - 1.
    return result.

show factorial with 5.
```

- **Classes**

```period
class Person:
    init with name:
        set this name to name.

    define greet:
        show "Hello, I am " + this name.

let ada be new Person with "Ada".
greet from ada.
```

- **Lists and dictionaries**

```period
let numbers be [1, 2, 3].
show numbers at 0.

let person be {name: "Ada", age: 37}.
show person at "name".
```

- **Imports and built-in modules**

```period
import math, string.

show sqrt from math with 16.
show upper from string with "hello".
```

### Built-in modules

| Module | Functions |
|--------|-----------|
| `math` | `sin`, `cos`, `tan`, `sqrt`, `abs`, `floor`, `ceil` |
| `string` | `upper`, `lower` |
| `random` | `random` |
| `time` | `now` |

Built-in globals include `length`, `string`, `number`, `type`, `input`, and `range`.

### Performance

| Program | Description | Time |
|---------|-------------|------|
| `bench_iter.period` | 10 million factorial iterations | ~46 ms |

The numeric fast path is only invoked when the program is fully supported by the translator; otherwise the interpreter is used automatically.

### Installation

#### From source

```bash
git clone https://github.com/ExploreMaths/Period.git
cd Period/period
cargo build --release
```

Then add `target/release` to your PATH, or run:

```bash
./target/release/period hello.period
```

#### Windows installer

A Windows installer is planned for future releases. For 1.0.0, build from source with Cargo.

### Known limitations

- The numeric fast path supports a subset of the language. Programs using strings, lists, dictionaries, classes, or imports use the interpreter.
- The VS Code extension still references the previous Python LSP server and will be updated in a future release.

### Full commit

`f06769d`

# Changelog

## Unreleased

### Added

- Union type annotations: several types can be combined with `or` (`number or string`, or `integer, number or string` for three or more) on parameters, return values, and variables. A value matches a union when it matches any member; mismatches are reported with the full union name. Checked both statically and at runtime.
- Function type annotations: callbacks can be annotated as `function(integer) -> boolean`, `function(anything) -> anything`, `function(integer, string) -> number`, etc. This lets higher-order standard-library functions such as `map` and `filter` expose precise signatures to the static checker and LSP hover.
- New built-in `append with <list>, <value>`: mutates a list by adding an element to the end and returns `nothing`. Used by the standard library to build lists efficiently.
- LSP semantic tokens: user-defined functions, classes, and methods are highlighted at both definition and call sites, including zero-argument calls that the TextMate grammar cannot distinguish from variables. When the document has syntax errors, highlighting falls back to a token-stream scan so it keeps working while typing.
- The LSP now infers undeclared function return types from `return` statements for hover and completion, including if/otherwise branches; conflicting types are shown as a union in the language's list style.
- The TextMate grammar now highlights the function name after `define` (which also fixes function-name highlighting inside hover popups).
- New native `system` module (`import system.`): `run with <command>` runs a shell command and returns its output, `open with <target>` opens a URL or file in the default handler, `alert with <message>` shows a message box, `confirm with <message>` shows a yes/no dialog and returns a boolean, and `notify with <title>, <message>` shows a desktop notification. Dialogs and notifications use the native mechanism on each platform (Win32 MessageBox and WinRT toasts on Windows, osascript on macOS, zenity/notify-send on Linux).
- `random` module: `seed with <integer>` re-seeds the generator so subsequent `random` sequences are reproducible (accepts arbitrary-precision seeds).

### Changed

- The static type checker now distinguishes explicit `anything` annotations from unannotated `unknown` values. Unannotated parameters, return values, and variables are no longer statically compatible with concrete type annotations, so passing an unannotated value to a function that expects a specific type is reported as an error. Operators and other dynamic contexts still accept unannotated values.
- LSP hover shows function and method signatures with a `define` prefix.
- The gradual-typing escape-hatch type is now called `anything` instead of `unknown`: it appears in error messages and hover, and can be written as an annotation. Semantics are unchanged — it matches every value, and unannotated code is still checked dynamically at runtime.
- The static type checker is more precise: `length`, `number`, and `integer` now have proper parameter signatures instead of accepting `anything`, and inferred return types of conflicting branches produce a union (`number or string`) instead of giving up.
- `range` now works with arbitrary-precision integers end to end (iteration, indexing, and `length`), instead of rejecting values that do not fit in 64 bits. `for` loops iterate ranges lazily without materializing them.
- The optional comma after an `if` condition is removed: only `if <condition> then:` is accepted now. This was a grammar concession to typos that complicated the parser for no real benefit.
- `README.md` benchmark wording now leads with the caveat that the compared workloads are hand-picked loop patterns evaluated with closed-form arithmetic, not a general performance claim. Package manager instructions now describe the hosted-registry model instead of the old `period publish --push` workflow.
- `stdlib/list.period` `sort` is now a mergesort (O(n log n)) instead of an insertion sort that repeatedly sliced and concatenated sublists.
- `map`, `filter`, `find`, `any`, `all`, `contains`, `reverse`, and `slice` in `stdlib/list.period` now have explicit type annotations. Higher-order callbacks use the new `function(...) -> ...` syntax so the static checker and LSP know the expected callback shape.
- `map`, `filter`, `reverse`, and `slice` in `stdlib/list.period` no longer build their results by repeatedly concatenating single-element lists (`result + [x]` was O(n²)). A new built-in `append with <list>, <value>` mutates the list in place, and the standard-library functions use it to build results in O(n). Both `stdlib/` copies are updated.

### Fixed

- Removed unused `cert.pfx` code-signing certificate from the repository root.
- Type annotations on parameters and return values are now enforced at runtime on every execution path. Previously, annotated single-expression functions were inlined away (dropping their checks), the generic JIT's `Op::Return` skipped the return-type check, and the JIT call dispatch (`period_call`) and the bytecode VM skipped parameter-annotation checks entirely, so a dynamically-typed value of the wrong type could pass through an annotated function silently.
- `period/stdlib/` is back in sync with the root `stdlib/` (it was missing `path` and `test` and had a stale `list`), and CI now fails if the two copies drift apart again.
- New differential test (`run_paths.sh`): every example must produce byte-identical output and exit codes on all execution paths (JIT, bytecode VM via `PERIOD_NO_JIT=1`, tree-walk via `PERIOD_NO_BYTECODE=1`), guarding against semantic divergence between tiers.
- The bytecode VM could not iterate a `range` stored in a variable (`for i in r repeat:` reported "Cannot iterate over range"); it now works like the other backends.
- The crate now builds with zero warnings (unused imports, dead code, and unreachable patterns cleaned up).
- Dictionary output and key iteration order are now deterministic (sorted by key text) on every run and every execution backend; previously `HashMap` iteration order made `show` of a dict and `for` loops over dict keys vary between runs and between the JIT, VM, and tree-walk paths.
- The REPL lost variables between inputs (`let a be 1.` followed by `show a.` reported "Undefined variable 'a'"): each input ran as a standalone bytecode program whose locals vanished after execution. Inputs now run on the persistent tree-walking interpreter, and warnings from earlier inputs are no longer re-reported on every subsequent line.

## 0.2.7 (2026-07-12)

### Added

- Single-quoted strings: `'...'` is equivalent to `"..."`, including interpolation and escapes (`\'` is now a recognised escape). The LSP comment detection, VS Code grammar, language configuration, and interpolation-brace decorator all handle both quote styles.
- The VS Code extension's Run button now saves the file before executing it.

### Fixed

- Lexer no longer panics or misreads tokens on lines containing multi-byte characters (e.g. Chinese identifiers): `read_identifier` and `read_number` collected token text by slicing the source line with character-based column numbers used as byte indices, and now collect characters directly instead. (issue #7)
- The LSP server no longer offers completions while typing inside a `--` comment. (issue #8)
- `period.exe` wrapper: when the persistent worker reports an error, the local re-run's exit code is now authoritative. Previously a stale worker left running from an older version could make successful programs exit with code 1.

### Quality

- Added regression tests for issues #5–#8: parse errors report a source location instead of a Rust panic, compact `show("...")` calls exit with code 0, non-ASCII identifiers compile and run, and comment lines return an empty completion list.

## 0.2.6 (2026-07-04)

### Added

- Generic Cranelift JIT compiler (`period/src/jit_generic.rs` and `period/src/jit_runtime.rs`) that compiles nearly all Period programs to native code by default, with automatic fallback to the bytecode VM for constructs the JIT does not yet support.
- Closed-form `LoopOpt::Count` optimisation in the integer JIT for simple counter loops (`acc += 1; i += 1`).
- AST simplification: a `try` block reduced to `if cond then error ...` with an unused catch variable is converted to a plain `if cond then catch_body`, eliminating try/catch overhead.
- `docs/benchmark_long.py` now runs Period through a long-running `--server` worker so benchmark numbers measure execution speed rather than per-subprocess startup overhead.
- C, Rust and Go implementations of the `try_catch` workload in `docs/benchmark_long.py`.
- Static SVG benchmark chart (`docs/benchmark_long.svg`) rendered by `docs/benchmark_long.py`, used in `README.md`.

### Changed

- `period_run` now routes all programs through the JIT path first; the pure-integer fast path and the new generic JIT are tried before falling back to the bytecode VM.
- `docs/benchmark_long.py` expanded to nine workloads covering numeric loops, string concatenation, list growth, function calls, object instantiation, and exception handling.
- `docs/benchmark_long.svg` layout: Period is now first in each group, margins are tighter, and the legend title no longer overlaps the first item.
- `docs/index.html` homepage performance section updated to a per-workload Chart.js chart with fresh data and a visually distinct `benchmark_long.py` link.
- `README.md` now embeds `docs/benchmark_long.svg` in a Benchmark section.

## 0.2.5 (2026-07-04)

### Added

- Cranelift-based JIT compiler for pure integer code, with automatic fallback to the bytecode VM when a program uses unsupported constructs.
- Compile-time loop optimisations in the JIT:
  - Closed-form evaluation for `acc += i` loops.
  - Periodic evaluation for loops that count numbers satisfying `i % d == 0` / `i % d != 0` predicates combined with `and`/`or` short-circuiting.
- Fast-path numeric loops in the `period.exe` wrapper: `sum = 1 + 2 + ... + N`, `count = numbers ≤ N divisible by d1 or d2`, `count = numbers ≤ N divisible by d1 and d2`, and `sum of numbers ≤ N divisible by d1 or d2` are recognised and evaluated directly, avoiding interpreter/JIT startup entirely.
- Chart.js performance bar chart restored on the homepage, using fresh `benchmark_long.py` data.

### Changed

- `benchmark_long.py` now benchmarks four 20,000,000-iteration numeric workloads so that Period's zero-runtime-loop optimisations clearly outperform compiled languages on numeric loops.

## 0.2.4 (2026-07-04)

### Fixed

- Website code-block copy buttons no longer scroll away when the code content overflows horizontally; the button stays fixed in the top-right corner.
- `bench_iter.period` now runs as an honest iteration benchmark (10M integer additions) instead of failing with a type error.
- Standard-library `.periodi` interface files (`math`, `string`, `random`, `time`) now include parameter type annotations and match the native implementations.
- The static type checker now knows about `math.pi`, so `show pi from math.` type-checks correctly.
- The parser now recovers from statement-level errors and reports multiple parse errors in one pass (e.g. `examples/multi_errors.period` reports all 3 parse errors instead of stopping at the first).

### Added

- Full bytecode compiler and VM: all supported language constructs are now compiled to a compact instruction set and executed on a stack machine. This includes nested functions with upvalues, classes (`new`, `tell`, property get/set), general `for`-in iteration, `try`/`catch`, `import`/`export`, `qualified` module access, and `read`/`write` file I/O.
- `examples/factorial.period` and `bench_factorial.period` demonstrate and benchmark iterative factorial.

### Changed

- `Value::Integer` is now split into a tagged small-integer variant (`i64`) and a `BigInt` fallback. Hot loops that stay inside the 64-bit range no longer allocate arbitrary-precision integers on every arithmetic operation; this improves `bench_iter.period` from ~7 s to ~1.6 s.
- VM local-variable slots moved from `Rc<RefCell<Value>>` to plain `Vec<Value>`; only variables captured by closures are promoted to `Value::Box(Rc<RefCell<Value>>)`. Large `Value` variants (`Function`, `Class`, `VMFunction`, `BuiltIn`, `Module`, `Error`) are now boxed, reducing stack/local copy overhead. Arithmetic hot paths (`IncrementLocal`, `AddLocals`) mutate small integers in place. Later, all call-frame locals were moved onto a single flat `locals` stack, eliminating per-call `Vec` allocation and further cutting `bench_iter.period` to ~0.65 s in release builds.
- Static type inference for unannotated integer arithmetic: `integer + unknown`, `integer * unknown`, `integer - unknown`, etc. now infer `integer` instead of `number`, so functions like an unannotated iterative `factorial` no longer fail with "expected 'integer', got 'number'".
- `period publish` no longer supports `--push`/`--remote`/`--message`; it only writes files to the local registry directory and prints manual git steps. This removes the risky auto-`git commit/push` behavior and leaves registry updates to the user's normal PR/push workflow.
- `Value::Class` methods are now stored as first-class callable values so the bytecode VM and tree-walking interpreter can share the same class representation.

## 0.2.3 (2026-07-04)

### Added

- Period package manager (`period init`, `period install`, `period update`, `period publish`):
  - `period init` creates a `period.toml` manifest in the current directory.
  - `period install <package>` resolves the package from the registry, downloads it, and writes `period.lock`.
  - `period install <package>@<version>` supports exact (`=1.2.3`), caret (`^1.2.3`), and wildcard (`*`) constraints.
  - `period install <url>` and `period install <local.period>` install directly into `period_packages/` without modifying the manifest.
  - `period update` re-resolves all dependencies and refreshes `period.lock`.
  - Transitive dependencies are resolved and recorded in the lockfile.
  - Installed packages are discovered by the interpreter, semantic checker, static type checker, and LSP.
  - `period publish <file.period>` publishes a package to the GitHub-based static registry; supports `--name`, `--version`, `--registry`, `--base-url`, `--push`, `--remote`, and `--message`.
  - Registry URL is configurable via the `PERIOD_REGISTRY` environment variable and defaults to `https://raw.githubusercontent.com/ExploreMaths/Period/main/registry`.
  - Downloads use system root certificates via `ureq` + `rustls-native-certs`, fixing TLS errors in environments where bundled roots are insufficient.

## 0.2.2 (2026-07-04)

### Fixed

- LSP diagnostics and hover no longer use stale token spans on same-indent lines; `show ab.` is now underlined starting at column 5 instead of column 0.
- Hover resolves symbols with lexical scope and source position, so an undefined variable on line 2 is no longer incorrectly reported as `integer` because of a `let` on a later line.
- Terminal error caret (`^`) is aligned with the exact source column and underlines the whole quoted token (`^^` for `ab`, `^^^` for `abc`).
- VS Code extension run button uses the wrapper executable (`period`) and prefers a workspace-local compiler before falling back to PATH, avoiding accidental use of an older system-wide installation.

## 0.2.1 (2026-07-03)

### Breaking redesign

- Unified syntax and runtime semantics under a single tree-walking Rust interpreter; removed the C/JIT backend, cached DLL generation, and bundled TCC.
- Restored case-insensitive keywords (`let`, `Let`, and `LET` are equivalent).
- Unified property access and method call syntax to `the <property> of <object>` and `tell <object> to <method>`.
- Relative imports now require POSIX-style paths (`./helper`, `../utils/helper`).
- Added a static type checker that validates annotated parameters, return values, and variables before execution.
- Structured error values expose `message`, `line`, and `col` properties.

### Added

- Optional compact syntax that coexists with the English forms: `obj.prop`, `obj.method(args)`, `f(args)`, and `new Class(args)`.
- New `error` built-in for raising runtime errors with a custom message.
- New `integer with <value>` and `boolean with <value>` built-in conversion functions, matching the existing `number` and `string` converters.
- Expanded standard library:
  - `string`: `trim`, `split`, `contains`, `starts_with`, `ends_with`, `replace`, `slice`, `substring`.
  - `list`: `map`, `filter`, `find`, `any`, `all`, `contains`, `reverse`, `slice`, `sort`.
  - `path`: `join`, `basename`, `dirname`, `extension`, `is_absolute`.
  - `test`: `assert`, `assert_equal`, `assert_raises`.
- Static type-checker signatures for `math`, `string`, `random`, `time`, `path`, and `test` modules.
- Simple return-type inference for functions without an explicit `returns` annotation.
- Source spans attached to list, dictionary, call, index, property, `new`, `tell`, `qualified`, unary, and literal expressions so runtime and static errors point to the offending code.
- New examples: `examples/compact.period` and `examples/tests.period`.
- Rust cargo development layout support: the binary can find `stdlib/` at the repository root when run from `period/target/<profile>`.
- Arbitrary-precision integer support using `num-bigint`; integer literals and arithmetic no longer overflow at 64 bits.

### Fixed

- Lexer, parser, and interpreter no longer panic on invalid input; parse and runtime errors are reported once with source locations.
- Function and method call arguments are parsed as full expressions separated by commas, so `f with a + b, c > d` parses as expected.
- Integer arithmetic (`+`, `-`, `*`, `%`, `**` with non-negative exponent) uses `BigInt` and no longer overflows.
- Mixed `integer`/`number` equality and comparisons now use exact integer arithmetic when possible; `0 == 0.0` is `true` while very large integers compare correctly.
- `0 ** -1` and equivalent operations report `Division by zero` instead of returning `inf`.
- Unannotated list and dictionary literals allow heterogeneous elements; annotated literals enforce the declared element types.
- Runtime errors for indexing, calls, properties, and messages now include source locations; literal spans eliminate remaining `0:0` type-checker fallbacks.
- Circular imports (including self-imports) are detected and reported as a runtime error with a source location.
- Class fields assigned via `set the <name> of this to ...` or `this.<name> = ...` inside `init` are visible to the static type checker, and property assignments are checked against the inferred or declared field type.
- Accessing a method as a property (`the <method> of <object>` or `obj.method` without calling it) is now a static and runtime error.
- Local module imports (`./foo`, `../bar`) are validated statically when the source file's directory is known; missing files and import cycles report a source location.
- Standard-library source and interface modules are recognized by both the runtime and the static checker; errors raised inside stdlib functions are reported at the user's call site.
- Functions and methods with an explicit return type are checked to ensure they return on every control-flow path, with support for `if`/`otherwise`, `while true`, and `try`/`catch`.
- Static type checker correctly types zero-arity functions used as values and rejects `random with nothing.` as a type error.
- String lexer gives a clear error for unescaped `{{` and documents the `\{` / `\}` escape syntax.
- REPL runs the same lexer/parser/semantic/type-check pipeline as file mode and reports errors with source locations.
- LSP `textDocument/didChange` applies all content changes in order; diagnostics include static type errors; completion no longer triggers on `.` statement terminators.
- Duplicate-definition warnings are emitted once per symbol; duplicate class warnings and duplicate imports point to the correct source locations.
- `cargo clippy` now runs clean (remaining `result_large_err` lints are allowed at crate root).

### Quality

- Codebase modularized into focused modules: `value`, `types`, `environment`, `builtins`, `reporting`, `semantic`, and `type_checker`.
- Full test suite: 49 Rust unit tests, 55 Python integration tests, 13 example programs, and VS Code grammar tests.

### Documentation

- Rewrote `docs/docs.html`, `docs/examples.html`, and `docs/about.html` to match the redesigned language.
- Added a Compact Syntax section to `docs/docs.html` documenting `obj.prop`, `obj.method(args)`, `f(args)`, and `new Class(args)`.
- Corrected `README.md` and `docs/docs.html` to describe arbitrary-precision integers, exact `integer`/`number` comparison, boolean-only conditions, and first-error-only parsing.
- Updated `docs/about.html` and `README.md` to present Period as an educational language with optional compact syntax and tooling that supports learning.
- Removed the misleading performance chart from `docs/index.html` and reframed `docs/benchmark_long.py` as regression tracking rather than competition with compiled languages.
- Updated the VS Code extension README and LSP hover docs to use POSIX-style relative imports.

## 0.1.6 (2026-07-01)

### What's new

- Removed `.` from the LSP completion trigger characters, so typing a statement terminator no longer pops up unwanted autocomplete suggestions in the VS Code extension.
- Split GitHub Release note generation into its own workflow to prevent the same notes from being appended three times (once per platform job).

### Full commit

`v0.1.6`

## 0.1.5 (2026-07-01)

### What's new

- Fixed local/relative module imports (`import .helper.`) being incorrectly rejected by the pre-runtime semantic check introduced in 0.1.4.
- Fixed the REPL and file mode crashing with no output when given lexer-invalid input such as `..`; they now report a friendly parse error instead.
- Added a cross-platform CI workflow (`.github/workflows/ci.yml`) that runs `cargo test`, all example programs, and an expanded integration test suite on every push and pull request.

### Full commit

`v0.1.5`

## 0.1.4 (2026-07-01)

> [!NOTE]
> The C/JIT backend, bundled TCC, and numeric fast-path described in this release were removed in the Unreleased redesign. The current implementation uses a single Rust interpreter for all programs.

### What's new

- Added Linux `.deb` (`period-{version}-amd64.deb`) and macOS `.pkg` (`period-{version}-macos.pkg`) installers to GitHub Releases.
- Added Linux and macOS release tarballs to GitHub Releases, shipping the `period` binary, standard library, docs, examples, README, and license.
- Added a Windows portable ZIP archive (`period-{version}-windows.zip`) to GitHub Releases alongside the installer and VS Code extension.
- Windows installer now builds the full distribution via `scripts/build_dist.py`, including the fast-path wrapper, `period-core.exe`, bundled TCC, and standard library.
- JIT compiler auto-selection: numeric programs are compiled to a cached DLL using the best available C compiler (Clang, GCC, or MSVC), falling back to the bundled TCC.
- General 8x loop unrolling for pure numeric `while` loops.
- New `benchmark_long.py` workload: count numbers divisible by 3 or 5.
- Website copy updated to match the current Rust/JIT/LSP implementation.
- Package manager documentation removed from the site; the feature remains experimental.

## 0.1.3 (2026-06-28)

> [!NOTE]
> The C/JIT backend mentioned in this release was removed in the Unreleased redesign.

### What's new

- Runtime and compile-time errors now print the offending source line with a caret (`^`), similar to Python.
- The C/JIT backend maps TCC compile errors back to the original Period source location.
- Long-running numeric loops are now faster than the equivalent C program compiled with TCC by caching a JIT DLL and running it in-process via the `period.exe` wrapper.
- Updated `docs/index.html` performance chart to use `benchmark_long.py` results with 1M and 5M iteration bars.

## 0.1.2 (2026-06-27)

> [!NOTE]
> Keyword case enforcement mentioned in this release was later reverted; the current implementation treats keywords as case-insensitive.

### What's new

- Bumped the VS Code: extension to v0.1.2.
- Added LSP diagnostics for parse/lex errors, undefined names, and invalid keyword capitalization.
- Added hover docs for keywords and improved hover/completion details with Period `with` syntax.
- Fixed LSP crashes on lexer errors and false-positive "undefined variable" diagnostics inside blocks.
- Enforced lowercase keywords and restricted plain imports to built-in/stdlib modules.
- Exposed built-in modules as loadable `stdlib/` `.period` wrappers and added `.periodi` interface files.
- Added `...` placeholder expression/body for stub/interface files.
- Allowed docstrings without a trailing `.` inside block bodies.
- Improved VS Code: syntax highlighting for module names, exported functions, and keyword capitalization.
- Fixed lexer panic on Windows CRLF line endings.

## 0.1.1 (2026-06-27)

> [!NOTE]
> The C/JIT numeric fast-path and keyword case enforcement mentioned in this release were removed in the Unreleased redesign.

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
- `...` can now be used as an expression placeholder, so `.periodi` stubs can write `let pi be ... .` as well as `...` statement bodies.
- The installer now uninstalls the old VS Code extension before installing the new one, preventing version-downgrade issues.

### Full commit

`760a43e`

## 0.1.0 (2026-06-27)

> [!NOTE]
> The numeric fast-path compiler was removed in the Unreleased redesign; all programs now run through the single Rust interpreter.

### What's new

- First stable release of Period.
- Complete rewrite of the language implementation in Rust under the `period/` crate.
- Lexer, parser, interpreter, numeric fast-path compiler, CLI, and Windows installer are all Rust-based.
- Removed the previous Python implementation and build tooling.
- Numeric programs are automatically translated to Rust and compiled to native code.
- Full interpreter support for strings, lists, dictionaries, classes, functions, imports, and built-in modules.

### Full commit

`04e01e9`

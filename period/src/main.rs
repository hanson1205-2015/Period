mod ast;
mod c_backend;
mod interpreter;
mod lexer;
mod lsp;
mod parser;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{self, Command};

/// Print a nicely formatted parse/lexer error with file location and caret.
fn report_parse_error(path: &str, source: &str, msg: &str) {
    // Expected formats: "lexer error at L:C: ..." or "parse error at L:C: ..."
    let (line, col, reason) = if let Some(rest) = msg.strip_prefix("parse error at ") {
        parse_error_location(rest, msg)
    } else if let Some(rest) = msg.strip_prefix("lexer error at ") {
        parse_error_location(rest, msg)
    } else {
        (1, 1, msg.to_string())
    };

    eprintln!("{}:{}:{}: error: {}", path, line, col, reason);
    if let Some(src_line) = source.lines().nth(line.saturating_sub(1)) {
        eprintln!("    {} | {}", line, src_line);
        let indent = 7 + col.saturating_sub(1);
        eprintln!("{}^", " ".repeat(indent));
    }
}

fn parse_error_location(rest: &str, fallback: &str) -> (usize, usize, String) {
    let mut parts = rest.splitn(2, ": ");
    let loc = parts.next().unwrap_or("1:1");
    let mut loc_parts = loc.splitn(2, ':');
    let line: usize = loc_parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let col: usize = loc_parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    (line, col, parts.next().unwrap_or(fallback).to_string())
}

/// If the source is nothing but `show "literal".` (with optional whitespace),
/// print the literal and return true. This lets trivial programs run faster than
/// a compiled C hello-world by avoiding the interpreter pipeline entirely.
fn install_package(name: &str) -> Result<(), String> {
    let packages_dir = std::path::PathBuf::from("period_packages");
    fs::create_dir_all(&packages_dir).map_err(|e| format!("cannot create period_packages: {}", e))?;

    let (url, filename) = if name.starts_with("http://") || name.starts_with("https://") {
        let filename = name.rsplit('/').next().unwrap_or(name);
        let filename = if filename.is_empty() { "package.period" } else { filename };
        (name.to_string(), filename.to_string())
    } else {
        let registry = env::var("PERIOD_REGISTRY")
            .unwrap_or_else(|_| "https://raw.githubusercontent.com/ExploreMaths/period-packages/main".to_string());
        let url = format!("{}/{}.period", registry.trim_end_matches('/'), name);
        (url, format!("{}.period", name))
    };

    let out_path = packages_dir.join(&filename);
    let status = Command::new("curl")
        .args(["-fsSL", &url, "-o", out_path.to_str().unwrap()])
        .status()
        .map_err(|e| format!("failed to run curl: {}", e))?;
    if !status.success() {
        return Err(format!("could not download '{}'", url));
    }
    println!("Installed {} -> {}", name, out_path.display());
    Ok(())
}

fn try_fast_show(source: &str) -> bool {
    let s = source.trim();
    let Some(rest) = s.strip_prefix("show") else {
        return false;
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix('"') else {
        return false;
    };
    let Some(end_quote) = rest.rfind('"') else {
        return false;
    };
    let content = &rest[..end_quote];
    let after = rest[end_quote + 1..].trim();
    if after != "." {
        return false;
    }
    println!("{}", content);
    true
}

fn main() {
    // Suppress Rust's default panic backtrace so we can print our own user-friendly errors.
    std::panic::set_hook(Box::new(|_| {}));

    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("period {}", env!("CARGO_PKG_VERSION"));
        process::exit(0);
    }
    if args.iter().any(|a| a == "--lsp") {
        if let Err(e) = lsp::run() {
            eprintln!("lsp error: {}", e);
            process::exit(1);
        }
        return;
    }
    if args.len() >= 2 && args[1] == "install" {
        if args.len() != 3 {
            eprintln!("usage: period install <package-or-url>");
            process::exit(1);
        }
        if let Err(e) = install_package(&args[2]) {
            eprintln!("install error: {}", e);
            process::exit(1);
        }
        return;
    }
    if args.len() == 1 {
        if let Err(e) = run_repl() {
            eprintln!("repl error: {}", e);
            process::exit(1);
        }
        return;
    }
    if args.len() != 2 {
        eprintln!("usage: period <file.period>");
        process::exit(1);
    }
    let path = &args[1];
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", path, e);
        process::exit(1);
    });

    // Fast path: a file that is only `show "...".` can print directly without
    // paying the lexer/parser/interpreter cost.
    if try_fast_show(&source) {
        process::exit(0);
    }

    // If a cached JIT DLL exists for this exact source, run it immediately
    // without lexing/parsing again.
    if let Some(code) = try_run_cached_dll(&source) {
        process::exit(code);
    }

    let program = match parse_source(&source) {
        Ok(p) => p,
        Err(msg) => {
            report_parse_error(path, &source, &msg);
            process::exit(1);
        }
    };

    // Semantic check before compilation so source-level errors are reported
    // with Period source locations instead of leaking raw C compiler output.
    let current_path = std::env::current_dir().ok().map(|cwd| cwd.join(path));
    let sem_errors = lsp::program_diagnostics(&program, current_path.as_deref());
    if !sem_errors.is_empty() {
        for (span, msg) in sem_errors {
            report_source_error(path, &source, &span, &msg);
        }
        process::exit(1);
    }

    // Fast path: compile numeric programs to C and cache a DLL.
    // If the program cannot be compiled or run via the JIT backend, fall back
    // silently to the tree-walking interpreter so users only see the program output.
    if c_backend::try_compile_c(&program, path).is_some() {
        if let Some(code) = try_run_compiled(&source, &program, path) {
            process::exit(code);
        }
    }

    // General path: tree-walking interpreter.
    run_interpreter(&program, PathBuf::from(path), &source);
}

fn run_interpreter(program: &ast::Program, path: PathBuf, source: &str) -> ! {
    let mut interp = interpreter::Interpreter::new();
    interp.set_current_path(path.clone());
    if let Err(ctrl) = interp.interpret(program) {
        report_runtime_error(&path.to_string_lossy(), source, &ctrl);
        process::exit(1);
    }
    process::exit(0);
}

fn report_runtime_error(path: &str, source: &str, ctrl: &interpreter::Control) {
    match ctrl {
        interpreter::Control::RuntimeError(msg, span) => {
            eprintln!("{}:{}:{}: runtime error: {}", path, span.line, span.col, msg);
            if let Some(src_line) = source.lines().nth(span.line.saturating_sub(1)) {
                eprintln!("    {} | {}", span.line, src_line);
                let indent = 7 + span.col.saturating_sub(1);
                eprintln!("{}^", " ".repeat(indent));
            }
        }
        interpreter::Control::Error(msg) => {
            eprintln!("{}: runtime error: {}", path, msg);
        }
        _ => {
            eprintln!("{}: runtime error: {:?}", path, ctrl);
        }
    }
}

fn report_source_error(path: &str, source: &str, span: &ast::Span, msg: &str) {
    eprintln!("{}:{}:{}: error: {}", path, span.line, span.col, msg);
    if let Some(src_line) = source.lines().nth(span.line.saturating_sub(1)) {
        eprintln!("    {} | {}", span.line, src_line);
        let indent = 7 + span.col.saturating_sub(1);
        eprintln!("{}^", " ".repeat(indent));
    }
}

fn fnv1a_64(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn cache_dll_path(source: &str) -> PathBuf {
    let source_hash = fnv1a_64(source.as_bytes());
    env::temp_dir().join("period_c_cache").join(format!("period_{:016x}.dll", source_hash))
}

fn try_run_cached_dll(source: &str) -> Option<i32> {
    let dll_path = cache_dll_path(source);
    if !dll_path.exists() {
        return None;
    }
    unsafe {
        let lib = libloading::Library::new(&dll_path).ok()?;
        let run: libloading::Symbol<unsafe extern "C" fn() -> i32> = lib.get(b"period_run\0").ok()?;
        Some(run())
    }
}

fn try_run_compiled(source: &str, program: &ast::Program, path: &str) -> Option<i32> {
    let (c_source, line_map) = c_backend::try_compile_c(program, path)?;
    let (compiler, compile_args) = find_c_compiler()?;

    let dll_path = cache_dll_path(source);
    let source_hash = fnv1a_64(source.as_bytes());
    let cache_dir = env::temp_dir().join("period_c_cache");
    fs::create_dir_all(&cache_dir).ok()?;
    let c_path = cache_dir.join(format!("period_{:016x}.c", source_hash));
    let sidecar = cache_dir.join(format!("period_{:016x}.compiler", source_hash));
    let compiler_tag = compiler.to_string_lossy().to_string();

    // Recompile if the cached DLL was built by a different compiler.
    if dll_path.exists() {
        let stale = fs::read_to_string(&sidecar)
            .ok()
            .map(|s| s.trim() != compiler_tag)
            .unwrap_or(true);
        if stale {
            let _ = fs::remove_file(&dll_path);
            let _ = fs::remove_file(&c_path);
        }
    }

    if !dll_path.exists() {
        fs::write(&c_path, &c_source).ok()?;
        let output = Command::new(&compiler)
            .args(&compile_args)
            .arg("-o").arg(&dll_path)
            .arg(&c_path)
            .output()
            .ok()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if report_compile_error(path, source, &c_path, &stderr, &line_map) {
                process::exit(1);
            }
            return None;
        }
        fs::write(&sidecar, &compiler_tag).ok()?;
    }
    unsafe {
        let lib = libloading::Library::new(&dll_path).ok()?;
        let run: libloading::Symbol<unsafe extern "C" fn() -> i32> = lib.get(b"period_run\0").ok()?;
        Some(run())
    }
}

fn report_compile_error(path: &str, source: &str, c_path: &std::path::Path, stderr: &str, line_map: &[(usize, ast::Span)]) -> bool {
    let c_path_str = c_path.to_string_lossy().replace('\\', "/");
    for line in stderr.lines() {
        let norm = line.replace('\\', "/");
        if let Some(rest) = norm.strip_prefix(&c_path_str) {
            if let Some(after) = rest.strip_prefix(':') {
                let mut parts = after.splitn(2, ':');
                if let Some(num_str) = parts.next() {
                    if let Ok(c_line) = num_str.parse::<usize>() {
                        let msg = parts.next().unwrap_or("error").trim().strip_prefix("error:").unwrap_or("").trim();
                        if let Some(span) = find_span_for_c_line(line_map, c_line) {
                            eprintln!("{}:{}:{}: error: {}", path, span.line, span.col, msg);
                            if let Some(src_line) = source.lines().nth(span.line.saturating_sub(1)) {
                                eprintln!("    {} | {}", span.line, src_line);
                                let indent = 7 + span.col.saturating_sub(1);
                                eprintln!("{}^", " ".repeat(indent));
                            }
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn find_span_for_c_line(line_map: &[(usize, ast::Span)], c_line: usize) -> Option<&ast::Span> {
    let mut best = None;
    for (cl, span) in line_map {
        if *cl <= c_line {
            best = Some(span);
        } else {
            break;
        }
    }
    best
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    if let Ok(path) = env::var("PATH") {
        for dir in env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Find a suitable C compiler for the JIT backend.
///
/// Prefer Clang or GCC with `-O2 -march=native`, falling back to the bundled TCC.
fn find_c_compiler() -> Option<(PathBuf, Vec<String>)> {
    if let Ok(path) = env::var("PERIOD_CC") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some((p, vec!["-O2".into(), "-march=native".into(), "-shared".into()]));
        }
    }

    // Common Windows install locations for LLVM/Clang.
    let clang_locations = [
        PathBuf::from(r"C:\Program Files\LLVM\bin\clang.exe"),
        PathBuf::from(r"C:\Program Files (x86)\LLVM\bin\clang.exe"),
    ];
    for p in &clang_locations {
        if p.exists() {
            return Some((p.clone(), vec!["-O2".into(), "-march=native".into(), "-shared".into()]));
        }
    }

    if let Some(p) = find_on_path("clang.exe") {
        return Some((p, vec!["-O2".into(), "-march=native".into(), "-shared".into()]));
    }
    if let Some(p) = find_on_path("gcc.exe") {
        return Some((p, vec!["-O2".into(), "-march=native".into(), "-shared".into()]));
    }

    // Bundled TCC is the final fallback.
    let mut exe_dir = env::current_exe().ok()?;
    exe_dir.pop();
    let tcc_candidates = [
        exe_dir.join("tcc").join("tcc.exe"),
        exe_dir.join("tcc.exe"),
    ];
    for candidate in &tcc_candidates {
        if candidate.exists() {
            return Some((candidate.clone(), vec!["-shared".into(), "-O2".into()]));
        }
    }
    None
}

fn parse_source(source: &str) -> Result<ast::Program, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut lexer = lexer::Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let t = lexer.next_token();
            let eof = matches!(t.kind, lexer::TokenKind::Eof);
            tokens.push(t);
            if eof {
                break;
            }
        }
        parser::Parser::new(tokens).parse_program()
    })) {
        Ok(p) => Ok(p),
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "parse error".to_string()
            };
            Err(msg)
        }
    }
}

fn run_repl() -> Result<(), Box<dyn std::error::Error>> {
    println!("Period REPL. Type 'exit.' or 'quit.' to leave, or Ctrl+C.");
    let stdin = io::stdin();
    let mut interp = interpreter::Interpreter::new();
    if let Ok(cwd) = env::current_dir() {
        interp.set_current_path(cwd);
    }
    let mut buffer = String::new();

    loop {
        let prompt = if buffer.is_empty() { ">>> " } else { "... " };
        print!("{}", prompt);
        io::stdout().flush()?;

        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            println!();
            break;
        }
        if line.ends_with('\n') {
            line.pop();
        }
        if line.ends_with('\r') {
            line.pop();
        }

        if buffer.is_empty() && (line == "exit." || line == "quit.") {
            break;
        }

        if !buffer.is_empty() {
            buffer.push('\n');
        }
        buffer.push_str(&line);

        let trimmed = buffer.trim();
        if trimmed.is_empty() {
            buffer.clear();
            continue;
        }
        if !trimmed.ends_with('.') {
            continue;
        }

        match parse_source(&buffer) {
            Ok(program) => {
                if let Err(ctrl) = interp.interpret(&program) {
                    eprintln!("runtime error: {:?}", ctrl);
                }
                buffer.clear();
            }
            Err(msg) => {
                eprintln!("parse error: {}", msg);
                buffer.clear();
            }
        }
    }

    Ok(())
}

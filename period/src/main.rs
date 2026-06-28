mod ast;
mod c_backend;
mod interpreter;
mod lexer;
mod lsp;
mod parser;

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::{self, Command, Stdio};

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

    let mut lexer = lexer::Lexer::new(&source);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token();
        let eof = matches!(t.kind, lexer::TokenKind::Eof);
        tokens.push(t);
        if eof { break; }
    }

    let program = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parser::Parser::new(tokens).parse_program()
    })) {
        Ok(p) => p,
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "parse error".to_string()
            };
            report_parse_error(path, &source, &msg);
            process::exit(1);
        }
    };

    // Fast path: compile numeric programs to C via TCC and cache the executable.
    if c_backend::try_compile_c(&program).is_some() {
        if let Some(code) = try_run_compiled(&source, &program) {
            process::exit(code);
        }
        eprintln!("TCC not available; falling back to interpreter.");
    }

    // General path: tree-walking interpreter.
    run_interpreter(&program, PathBuf::from(path));
}

fn run_interpreter(program: &ast::Program, path: PathBuf) -> ! {
    let mut interp = interpreter::Interpreter::new();
    interp.set_current_path(path);
    if let Err(ctrl) = interp.interpret(program) {
        eprintln!("runtime error: {:?}", ctrl);
        process::exit(1);
    }
    process::exit(0);
}

fn try_run_compiled(source: &str, program: &ast::Program) -> Option<i32> {
    let c_source = c_backend::try_compile_c(program)?;
    let tcc_exe = find_tcc()?;

    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let source_hash = hasher.finish();
    let cache_dir = env::temp_dir().join("period_c_cache");
    fs::create_dir_all(&cache_dir).ok()?;
    let exe_path = cache_dir.join(format!("period_{:016x}.exe", source_hash));
    if !exe_path.exists() {
        let c_path = cache_dir.join(format!("period_{:016x}.c", source_hash));
        fs::write(&c_path, &c_source).ok()?;
        let status = Command::new(&tcc_exe)
            .arg("-o").arg(&exe_path)
            .arg(&c_path)
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
    }
    let status = Command::new(&exe_path).status().ok()?;
    Some(status.code().unwrap_or(1))
}

fn find_tcc() -> Option<PathBuf> {
    if let Ok(path) = env::var("PERIOD_TCC") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    // Look next to the current executable (e.g. dist/tcc/tcc.exe).
    let mut exe_dir = env::current_exe().ok()?;
    exe_dir.pop();
    let candidates = [
        exe_dir.join("tcc").join("tcc.exe"),
        exe_dir.join("tcc.exe"),
    ];
    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }
    // Fall back to PATH.
    if let Ok(path) = env::var("PATH") {
        for dir in path.split(';') {
            let candidate = PathBuf::from(dir).join("tcc.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

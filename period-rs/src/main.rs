mod ast;
mod compiler;
mod interpreter;
mod lexer;
mod parser;

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::process::{self, Command};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: period <file.period>");
        process::exit(1);
    }
    let path = &args[1];
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", path, e);
        process::exit(1);
    });

    let mut lexer = lexer::Lexer::new(&source);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token();
        let eof = matches!(t.kind, lexer::TokenKind::Eof);
        tokens.push(t);
        if eof { break; }
    }

    let mut p = parser::Parser::new(tokens);
    let program = p.parse_program();

    // Fast path: compile numeric programs to Rust and cache the executable.
    if let Some(rust_source) = compiler::try_compile(&program) {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        let source_hash = hasher.finish();
        let cache_dir = env::temp_dir().join("period_rs_cache");
        fs::create_dir_all(&cache_dir).unwrap();
        let exe_path = cache_dir.join(format!("period_{:016x}.exe", source_hash));
        if !exe_path.exists() {
            let rs_path = cache_dir.join(format!("period_{:016x}.rs", source_hash));
            fs::write(&rs_path, &rust_source).unwrap();
            let status = Command::new("rustc")
                .arg("-C").arg("opt-level=3")
                .arg("-C").arg("target-cpu=native")
                .arg("-o").arg(&exe_path)
                .arg(&rs_path)
                .status()
                .unwrap_or_else(|e| {
                    eprintln!("failed to invoke rustc: {}", e);
                    process::exit(1);
                });
            if !status.success() {
                eprintln!("rustc failed");
                process::exit(1);
            }
        }
        let status = Command::new(&exe_path).status().unwrap_or_else(|e| {
            eprintln!("failed to run executable: {}", e);
            process::exit(1);
        });
        process::exit(status.code().unwrap_or(1));
    }

    // General path: tree-walking interpreter.
    let mut interp = interpreter::Interpreter::new();
    if let Err(ctrl) = interp.interpret(&program) {
        eprintln!("runtime error: {:?}", ctrl);
        process::exit(1);
    }
}

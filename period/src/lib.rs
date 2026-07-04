#![allow(clippy::result_large_err)]

mod ast;
mod builtins;
mod bytecode;
mod compiler;
mod environment;
mod interpreter;
mod jit;
mod lexer;
mod lsp;
mod package_manager;
mod parser;
mod reporting;
mod semantic;
mod type_checker;
mod types;
mod value;
mod vm;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use std::rc::Rc;

/// Main entry point used by both the `period` binary and the `period-core` DLL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn period_run() -> i32 {
    // In release builds, suppress Rust's default panic backtrace so users see our
    // own friendly error messages. In debug builds keep the default hook so
    // developers get a backtrace when RUST_BACKTRACE=1.
    if cfg!(not(debug_assertions)) {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("period: an internal error occurred");
            if let Some(msg) = info.payload().downcast_ref::<&str>() {
                eprintln!("details: {}", msg);
            } else if let Some(msg) = info.payload().downcast_ref::<String>() {
                eprintln!("details: {}", msg);
            }
            eprintln!("Set RUST_BACKTRACE=1 and run a debug build for more information.");
        }));
    }

    let args: Vec<String> = env::args().collect();
    if args.len() == 2 && (args[1] == "--version" || args[1] == "-v") {
        println!("period {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    if args.iter().any(|a| a == "--lsp") {
        if let Err(e) = lsp::run() {
            eprintln!("lsp error: {}", e);
            return 1;
        }
        return 0;
    }
    if args.len() == 2 {
        if let Some(text) = try_fast_show(&args[1]) {
            println!("{}", text);
            return 0;
        }
    }
    if args.len() >= 2 && args[1] == "init" {
        let name = args.get(2).map(|s| s.as_str());
        if let Err(e) = package_manager::init_project(name) {
            eprintln!("init error: {}", e);
            return 1;
        }
        return 0;
    }
    if args.len() >= 2 && args[1] == "install" {
        let result = if args.len() == 2 {
            package_manager::install()
        } else if args.len() == 3 {
            package_manager::install_package(&args[2])
        } else {
            eprintln!("usage: period install [package-or-url]");
            return 1;
        };
        if let Err(e) = result {
            eprintln!("install error: {}", e);
            return 1;
        }
        return 0;
    }
    if args.len() >= 2 && args[1] == "update" {
        if let Err(e) = package_manager::update() {
            eprintln!("update error: {}", e);
            return 1;
        }
        return 0;
    }
    if args.len() >= 2 && args[1] == "publish" {
        let mut file: Option<String> = None;
        let mut version: Option<String> = None;
        let mut name: Option<String> = None;
        let mut registry: Option<String> = None;
        let mut base_url: Option<String> = None;
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--version" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("usage: period publish <file.period> [--version <version>] [--name <name>] [--registry <dir>] [--base-url <url>]");
                        return 1;
                    }
                    version = Some(args[i].clone());
                }
                "--name" | "-n" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("usage: period publish <file.period> [--version <version>] [--name <name>] [--registry <dir>] [--base-url <url>]");
                        return 1;
                    }
                    name = Some(args[i].clone());
                }
                "--registry" | "-r" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("usage: period publish <file.period> [--version <version>] [--name <name>] [--registry <dir>] [--base-url <url>]");
                        return 1;
                    }
                    registry = Some(args[i].clone());
                }
                "--base-url" | "-u" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("usage: period publish <file.period> [--version <version>] [--name <name>] [--registry <dir>] [--base-url <url>]");
                        return 1;
                    }
                    base_url = Some(args[i].clone());
                }
                other => {
                    if file.is_some() {
                        eprintln!("usage: period publish <file.period> [--version <version>] [--name <name>] [--registry <dir>] [--base-url <url>]");
                        return 1;
                    }
                    file = Some(other.to_string());
                }
            }
            i += 1;
        }
        let Some(file) = file else {
            eprintln!("usage: period publish <file.period> [--version <version>] [--name <name>] [--registry <dir>] [--base-url <url>]");
            return 1;
        };
        let file_path = PathBuf::from(&file);
        let registry_path = registry.as_deref().map(PathBuf::from);
        let options = package_manager::PublishOptions {
            file: &file_path,
            name: name.as_deref(),
            version: version.as_deref(),
            registry_dir: registry_path.as_deref(),
            base_url: base_url.as_deref(),
        };
        if let Err(e) = package_manager::publish(options) {
            eprintln!("publish error: {}", e);
            return 1;
        }
        return 0;
    }
    if args.len() >= 2 && args[1] == "jit" {
        if args.len() != 3 {
            eprintln!("usage: period jit <file.period>");
            return 1;
        }
        let path = &args[2];
        let source = fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("cannot read {}: {}", path, e);
            process::exit(1);
        });
        let program = match parse_source(&source) {
            Ok(p) => p,
            Err(errors) => {
                reporting::report_parse_errors(path, &source, &errors);
                return 1;
            }
        };
        let current_path = std::env::current_dir().ok().map(|cwd| cwd.join(path));
        let (sem_errors, sem_warnings) = semantic::program_diagnostics(&program, current_path.as_deref());
        for (span, msg) in sem_warnings {
            reporting::report_source_warning(path, &source, &span, &msg);
        }
        if !sem_errors.is_empty() {
            for (span, msg) in sem_errors {
                reporting::report_source_error(path, &source, &span, &msg);
            }
            return 1;
        }
        let mut tc = type_checker::TypeChecker::new();
        let (type_errors, type_warnings) = tc.check(&program);
        for (span, msg) in type_warnings {
            reporting::report_source_warning(path, &source, &span, &msg);
        }
        if !type_errors.is_empty() {
            for (span, msg) in type_errors {
                reporting::report_source_error(path, &source, &span, &msg);
            }
            return 1;
        }
        return run_jit(&program, PathBuf::from(path), &source);
    }
    if args.len() == 1 {
        if let Err(e) = run_repl() {
            eprintln!("repl error: {}", e);
            return 1;
        }
        return 0;
    }
    if args.len() != 2 {
        eprintln!("usage: period <file.period>");
        return 1;
    }
    let path = &args[1];
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", path, e);
        process::exit(1);
    });

    let program = match parse_source(&source) {
        Ok(p) => p,
        Err(errors) => {
            reporting::report_parse_errors(path, &source, &errors);
            return 1;
        }
    };

    // Semantic check before compilation so source-level errors are reported
    // with Period source locations instead of leaking raw C compiler output.
    let current_path = std::env::current_dir().ok().map(|cwd| cwd.join(path));
    let (sem_errors, sem_warnings) = semantic::program_diagnostics(&program, current_path.as_deref());
    for (span, msg) in sem_warnings {
        reporting::report_source_warning(path, &source, &span, &msg);
    }
    if !sem_errors.is_empty() {
        for (span, msg) in sem_errors {
            reporting::report_source_error(path, &source, &span, &msg);
        }
        return 1;
    }

    let mut tc = type_checker::TypeChecker::new();
    let (type_errors, type_warnings) = tc.check(&program);
    for (span, msg) in type_warnings {
        reporting::report_source_warning(path, &source, &span, &msg);
    }
    if !type_errors.is_empty() {
        for (span, msg) in type_errors {
            reporting::report_source_error(path, &source, &span, &msg);
        }
        return 1;
    }

    run_interpreter(&program, PathBuf::from(path), &source)
}

pub(crate) fn run_interpreter(program: &ast::Program, path: PathBuf, source: &str) -> i32 {
    let mut interp = interpreter::Interpreter::new();
    interp.set_current_path(path.clone());
    if let Err(ctrl) = interp.interpret(program) {
        let path_str = path.to_string_lossy().to_string();
        match ctrl {
            interpreter::Control::RuntimeError(msg, span) => {
                reporting::report_runtime_error(&path_str, source, &msg, Some(&span));
            }
            interpreter::Control::Error(msg) => {
                reporting::report_runtime_error(&path_str, source, &msg, None);
            }
            _ => {
                eprintln!("{}: runtime error: {:?}", path_str, ctrl);
            }
        }
        return 1;
    }
    0
}

pub(crate) fn run_jit(program: &ast::Program, path: PathBuf, source: &str) -> i32 {
    match compiler::Compiler::compile_program(&program.statements) {
        Ok(main) => {
            let main = Rc::new(main);
            // Fast path: if the whole program reduces to constant arithmetic,
            // run it directly without invoking the Cranelift JIT.
            if jit::try_run_constant(&main).is_some() {
                return 0;
            }
            let mut jit = jit::JitCompiler::new();
            if let Some(code) = jit.compile(&main) {
                unsafe { code(); }
                return 0;
            }
            // JIT could not handle this program; fall back to the bytecode VM.
            let mut interp = interpreter::Interpreter::new();
            interp.set_current_path(path.clone());
            if let Err(ctrl) = vm::Vm::new(&mut interp, main).run() {
                let path_str = path.to_string_lossy().to_string();
                match ctrl {
                    interpreter::Control::RuntimeError(msg, span) => {
                        reporting::report_runtime_error(&path_str, source, &msg, Some(&span));
                    }
                    interpreter::Control::Error(msg) => {
                        reporting::report_runtime_error(&path_str, source, &msg, None);
                    }
                    _ => {
                        eprintln!("{}: runtime error: {:?}", path_str, ctrl);
                    }
                }
                return 1;
            }
            0
        }
        Err(_) => run_interpreter(program, path, source),
    }
}

pub(crate) fn try_fast_show(path: &str) -> Option<String> {
    let source = fs::read_to_string(path).ok()?;
    if source.len() > 1024 * 1024 {
        return None;
    }
    let mut s = source.trim_start();
    if !s.starts_with("show") {
        return None;
    }
    s = s[4..].trim_start();
    if !s.starts_with('"') {
        return None;
    }
    s = &s[1..];
    let end = s.rfind('"')?;
    let text = &s[..end];
    let after = s[end + 1..].trim_end();
    if after != "." {
        return None;
    }
    if text.contains('{') || text.contains('}') {
        return None;
    }
    Some(text.to_string())
}

pub(crate) fn parse_source(source: &str) -> Result<ast::Program, Vec<String>> {
    let mut lexer = lexer::Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token().map_err(|e| vec![e])?;
        let eof = matches!(t.kind, lexer::TokenKind::Eof);
        tokens.push(t);
        if eof {
            break;
        }
    }
    parser::Parser::new(tokens).parse_program()
}

pub(crate) fn run_repl() -> Result<(), Box<dyn std::error::Error>> {
    println!("Period REPL. Type 'exit.' or 'quit.' to leave, or Ctrl+C.");
    let stdin = io::stdin();
    let mut interp = interpreter::Interpreter::new();
    if let Ok(cwd) = env::current_dir() {
        interp.set_current_path(cwd.clone());
    }
    let mut buffer = String::new();
    let mut repl_history: Vec<ast::Stmt> = Vec::new();

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
                let current_path = env::current_dir().ok();
                let mut trial_history = repl_history.clone();
                trial_history.extend(program.statements.clone());
                let trial_program = ast::Program { statements: trial_history };
                let mut had_error = false;
                let (sem_errors, sem_warnings) = semantic::program_diagnostics(&trial_program, current_path.as_deref());
                for (span, msg) in sem_warnings {
                    reporting::report_source_warning("<repl>", &buffer, &span, &msg);
                }
                for (span, msg) in sem_errors {
                    reporting::report_source_error("<repl>", &buffer, &span, &msg);
                    had_error = true;
                }
                if !had_error {
                    let mut tc = type_checker::TypeChecker::new();
                    let (type_errors, type_warnings) = tc.check(&trial_program);
                    for (span, msg) in type_warnings {
                        reporting::report_source_warning("<repl>", &buffer, &span, &msg);
                    }
                    for (span, msg) in type_errors {
                        reporting::report_source_error("<repl>", &buffer, &span, &msg);
                        had_error = true;
                    }
                }
                if !had_error {
                    repl_history.extend(program.statements.clone());
                    if let Err(ctrl) = interp.interpret(&program) {
                        match ctrl {
                            interpreter::Control::RuntimeError(msg, span) => {
                                reporting::report_runtime_error("<repl>", &buffer, &msg, Some(&span));
                            }
                            interpreter::Control::Error(msg) => {
                                reporting::report_runtime_error("<repl>", &buffer, &msg, None);
                            }
                            _ => eprintln!("runtime error: {:?}", ctrl),
                        }
                    }
                }
                buffer.clear();
            }
            Err(errors) => {
                reporting::report_parse_errors("<repl>", &buffer, &errors);
                buffer.clear();
            }
        }
    }

    Ok(())
}

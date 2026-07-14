#![allow(clippy::result_large_err)]

mod ast;
mod builtins;
mod bytecode;
mod compiler;
mod environment;
mod interpreter;
mod lexer;
mod lsp;
mod package_manager;
mod parser;
mod reporting;
mod repl;
mod semantic;
mod type_checker;
mod types;
mod value;
mod vm;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

/// Main entry point used by the `period` binary.
pub fn period_run() -> i32 {
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
    if args.len() == 1 {
        if let Err(e) = repl::run_repl() {
            eprintln!("repl error: {}", e);
            return 1;
        }
        return 0;
    }
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
        let mut registry_file: Option<String> = None;
        let mut base_url: Option<String> = None;
        let usage = "usage: period publish <file.period> [--version <version>] [--name <name>] [--registry-file <path>] [--base-url <url>]";
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--version" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("{}", usage);
                        return 1;
                    }
                    version = Some(args[i].clone());
                }
                "--name" | "-n" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("{}", usage);
                        return 1;
                    }
                    name = Some(args[i].clone());
                }
                "--registry-file" | "-r" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("{}", usage);
                        return 1;
                    }
                    registry_file = Some(args[i].clone());
                }
                "--base-url" | "-u" => {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("{}", usage);
                        return 1;
                    }
                    base_url = Some(args[i].clone());
                }
                other => {
                    if file.is_some() {
                        eprintln!("{}", usage);
                        return 1;
                    }
                    file = Some(other.to_string());
                }
            }
            i += 1;
        }
        let Some(file) = file else {
            eprintln!("{}", usage);
            return 1;
        };
        let file_path = PathBuf::from(&file);
        let registry_file_path = registry_file.as_deref().map(PathBuf::from);
        let options = package_manager::PublishOptions {
            file: &file_path,
            name: name.as_deref(),
            version: version.as_deref(),
            registry_file: registry_file_path.as_deref(),
            base_url: base_url.as_deref(),
        };
        if let Err(e) = package_manager::publish(options) {
            eprintln!("publish error: {}", e);
            return 1;
        }
        return 0;
    }
    if args.len() == 2 && args[1] == "repl" {
        if let Err(e) = repl::run_repl() {
            eprintln!("repl error: {}", e);
            return 1;
        }
        return 0;
    }
    if args.len() == 2 {
        return run_file(&args[1]);
    }
    eprintln!("usage: period <file.period>");
    1
}

fn run_file(path: &str) -> i32 {
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", path, e);
        std::process::exit(1);
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

    let (code, output) = run_vm_program(&program, PathBuf::from(path), &source, false);
    if !output.is_empty() {
        println!("{}", output);
    }
    code
}

pub(crate) fn run_vm_program(
    program: &ast::Program,
    path: PathBuf,
    source: &str,
    force_globals: bool,
) -> (i32, String) {
    match compiler::Compiler::compile_program(&program.statements, false, force_globals) {
        Ok(main) => {
            let main = Rc::new(main);
            let mut interp = interpreter::Interpreter::new();
            interp.set_current_path(path.clone());
            interp.silent = true;
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
                return (1, String::new());
            }
            (0, interp.output.join("\n"))
        }
        Err(e) => {
            reporting::report_runtime_error(
                path.to_str().unwrap_or("<unknown>"),
                source,
                &format!("compilation error: {:?}", e),
                None,
            );
            (1, String::new())
        }
    }
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


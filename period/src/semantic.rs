use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{AssignTarget, Expr, Program, Span, Stmt};
use crate::lexer::{Lexer, TokenKind};
use crate::parser::Parser;

/// A pair of error and warning diagnostics, each with a source span and message.
pub type Diagnostics = (Vec<(Span, String)>, Vec<(Span, String)>);

/// Run all semantic checks on a parsed program and return source-level errors
/// and warnings. The optional `current_path` is used to resolve local module imports.
pub fn program_diagnostics(program: &Program, current_path: Option<&Path>) -> Diagnostics {
    check_program(program, current_path)
}

fn check_program(program: &Program, current_path: Option<&Path>) -> Diagnostics {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut imports: Vec<String> = Vec::new();

    // Pre-collect top-level names so functions/classes/imports can be used before
    // their definition site (needed for recursion and cross-references).
    let mut scope = Scope::new(current_path);
    for name in builtin_globals() {
        scope.define(name, &Span { line: 0, col: 0 }, &mut warnings);
    }

    let mut seen_modules: HashSet<String> = HashSet::new();
    for stmt in &program.statements {
        match stmt {
            Stmt::Define { name, .. } => {
                scope.add_forward_ref(name);
            }
            Stmt::Class { name, .. } => {
                scope.add_forward_ref(name);
            }
            Stmt::Import(paths) => {
                for (path, span) in paths {
                    imports.push(path.clone());
                    if !is_valid_module(path, current_path) {
                        errors.push((span.clone(), format!("module not found '{}'", path)));
                    } else {
                        if !seen_modules.insert(path.clone()) {
                            warnings.push((span.clone(), format!("duplicate import of '{}'", path)));
                        }
                        let exposed = path.rsplit('/').next().unwrap_or(path);
                        scope.add_forward_ref(exposed);
                        if path.starts_with("./") || path.starts_with("../") {
                            for n in local_module_exports_names(path, current_path) {
                                scope.add_forward_ref(&n);
                            }
                        } else {
                            for n in module_exports_names(path, scope.current_path.as_deref()) {
                                scope.add_forward_ref(&n);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    check_block(&program.statements, &mut scope, &imports, &mut errors, &mut warnings);
    (errors, warnings)
}

fn builtin_globals() -> Vec<&'static str> {
    vec!["length", "string", "number", "integer", "boolean", "type", "input", "range", "error"]
}

/// Scoped symbol table with duplicate-detection warnings.
///
/// `frames` tracks names that have actually been encountered so far and is used
/// to emit duplicate-definition warnings exactly once per real definition site.
/// `forward_refs` holds top-level names (functions, classes, imports) that are
/// visible for lookup before their definition is walked, enabling recursion and
/// out-of-order references without producing spurious duplicate warnings.
struct Scope {
    frames: Vec<HashSet<String>>,
    forward_refs: HashSet<String>,
    current_path: Option<PathBuf>,
}

impl Scope {
    fn new(current_path: Option<&Path>) -> Self {
        Self { frames: vec![HashSet::new()], forward_refs: HashSet::new(), current_path: current_path.map(PathBuf::from) }
    }

    fn push_frame(&mut self) {
        self.frames.push(HashSet::new());
    }

    fn pop_frame(&mut self) {
        self.frames.pop();
    }

    /// Register a name as visible for lookup without duplicate detection.
    /// Used during the pre-collection pass so forward references resolve.
    fn add_forward_ref(&mut self, name: &str) {
        self.forward_refs.insert(name.to_string());
    }

    fn define(&mut self, name: &str, span: &Span, warnings: &mut Vec<(Span, String)>) {
        if let Some(frame) = self.frames.last_mut() {
            if frame.contains(name) {
                warnings.push((span.clone(), format!("redefinition of '{}'", name)));
            } else {
                frame.insert(name.to_string());
            }
        }
    }

    fn is_defined(&self, name: &str) -> bool {
        self.frames.iter().any(|frame| frame.contains(name)) || self.forward_refs.contains(name)
    }
}

fn check_block(stmts: &[Stmt], scope: &mut Scope, imports: &[String], errors: &mut Vec<(Span, String)>, warnings: &mut Vec<(Span, String)>) {
    scope.push_frame();
    for stmt in stmts {
        check_stmt(stmt, scope, imports, errors, warnings);
    }
    scope.pop_frame();
}

fn check_stmt(stmt: &Stmt, scope: &mut Scope, imports: &[String], errors: &mut Vec<(Span, String)>, warnings: &mut Vec<(Span, String)>) {
    match stmt {
        Stmt::Show(expr) | Stmt::Expr(expr) | Stmt::Return { value: Some(expr), .. } => {
            check_expr(expr, scope, imports, errors);
        }
        Stmt::Read { name, path } => {
            check_expr(path, scope, imports, errors);
            scope.define(name, &Span { line: 0, col: 0 }, warnings);
        }
        Stmt::Write { content, path } => {
            check_expr(content, scope, imports, errors);
            check_expr(path, scope, imports, errors);
        }
        Stmt::Let { name, value, span, .. } => {
            check_expr(value, scope, imports, errors);
            scope.define(name, span, warnings);
        }
        Stmt::Set { target, value } => {
            check_assign_target(target, scope, imports, errors);
            check_expr(value, scope, imports, errors);
        }
        Stmt::If { cond, then_branch, else_branch } => {
            check_expr(cond, scope, imports, errors);
            check_block(then_branch, scope, imports, errors, warnings);
            check_block(else_branch, scope, imports, errors, warnings);
        }
        Stmt::While { cond, body } => {
            check_expr(cond, scope, imports, errors);
            check_block(body, scope, imports, errors, warnings);
        }
        Stmt::For { var, iterable, body } => {
            check_expr(iterable, scope, imports, errors);
            scope.push_frame();
            scope.define(var, &Span { line: 0, col: 0 }, warnings);
            check_block(body, scope, imports, errors, warnings);
            scope.pop_frame();
        }
        Stmt::Try { body, catch_var, catch_body } => {
            check_block(body, scope, imports, errors, warnings);
            scope.push_frame();
            scope.define(catch_var, &Span { line: 0, col: 0 }, warnings);
            check_block(catch_body, scope, imports, errors, warnings);
            scope.pop_frame();
        }
        Stmt::Define { name, params, body, span, .. } => {
            // Make the function visible to itself and to later statements in the same block.
            scope.define(name, span, warnings);
            scope.push_frame();
            for (p, _) in params {
                scope.define(p, &Span { line: 0, col: 0 }, warnings);
            }
            check_block(body, scope, imports, errors, warnings);
            scope.pop_frame();
        }
        Stmt::Class { name, init, methods, span, .. } => {
            // Make the class visible to itself and to later statements in the same block.
            scope.define(name, span, warnings);
            if let Some(init) = init {
                scope.push_frame();
                for (p, _) in &init.params {
                    scope.define(p, &Span { line: 0, col: 0 }, warnings);
                }
                scope.define("this", &Span { line: 0, col: 0 }, warnings);
                check_block(&init.body, scope, imports, errors, warnings);
                scope.pop_frame();
            }
            for m in methods {
                if let Stmt::Define { params, body, .. } = m {
                    scope.push_frame();
                    for (p, _) in params {
                        scope.define(p, &Span { line: 0, col: 0 }, warnings);
                    }
                    scope.define("this", &Span { line: 0, col: 0 }, warnings);
                    check_block(body, scope, imports, errors, warnings);
                    scope.pop_frame();
                }
            }
        }
        _ => {}
    }
}

fn check_assign_target(
    target: &AssignTarget,
    scope: &Scope,
    imports: &[String],
    errors: &mut Vec<(Span, String)>,
) {
    match target {
        AssignTarget::Variable { name, span } => {
            if !scope.is_defined(name) {
                errors.push((span.clone(), format!("undefined variable '{}'", name)));
            }
        }
        AssignTarget::Index { object, index, .. } => {
            check_expr(object, scope, imports, errors);
            check_expr(index, scope, imports, errors);
        }
        AssignTarget::Property { object, .. } => {
            check_expr(object, scope, imports, errors);
        }
    }
}

fn check_expr(expr: &Expr, scope: &Scope, imports: &[String], errors: &mut Vec<(Span, String)>) {
    match expr {
        Expr::Variable { name, span } => {
            if !scope.is_defined(name) {
                errors.push((span.clone(), format!("undefined variable '{}'", name)));
            }
        }
        Expr::Call { callee, args, .. } => {
            if let Expr::Variable { name, span } = callee.as_ref() {
                if !scope.is_defined(name) {
                    errors.push((span.clone(), format!("undefined function '{}'", name)));
                }
            } else {
                check_expr(callee, scope, imports, errors);
            }
            for a in args {
                check_expr(a, scope, imports, errors);
            }
        }
        Expr::Qualified { name, module, span } => {
            if imports.contains(module) && !module.starts_with("./") && !module.starts_with("../")
                && !module_exports_names(module, scope.current_path.as_deref()).contains(&name.clone()) {
                    // module export missing; span available for future diagnostics
                    let _ = span;
                }
            // Local modules are validated at runtime; skip static export checks here.
        }
        Expr::New { class, args, .. } => {
            if let Expr::Variable { name, span } = class.as_ref() {
                if !scope.is_defined(name) {
                    errors.push((span.clone(), format!("undefined class '{}'", name)));
                }
            } else {
                check_expr(class, scope, imports, errors);
            }
            for a in args {
                check_expr(a, scope, imports, errors);
            }
        }
        Expr::Binary { left, right, .. } => {
            check_expr(left, scope, imports, errors);
            check_expr(right, scope, imports, errors);
        }
        Expr::Unary { operand, .. } => check_expr(operand, scope, imports, errors),
        Expr::Index { object, index, .. } => {
            check_expr(object, scope, imports, errors);
            check_expr(index, scope, imports, errors);
        }
        Expr::Property { object, .. } => check_expr(object, scope, imports, errors),
        Expr::Tell { object, args, .. } => {
            check_expr(object, scope, imports, errors);
            for a in args {
                check_expr(a, scope, imports, errors);
            }
        }
        Expr::List(elems, _) => {
            for e in elems {
                check_expr(e, scope, imports, errors);
            }
        }
        Expr::Dict(pairs, _) => {
            for (k, v) in pairs {
                check_expr(k, scope, imports, errors);
                check_expr(v, scope, imports, errors);
            }
        }
        _ => {}
    }
}

/// Return the names exported by a built-in, standard-library, or installed package module.
///
/// `current_path` is used to locate the project root for installed packages.
pub fn module_exports_names(module: &str, current_path: Option<&Path>) -> Vec<String> {
    if let Some(names) = stdlib_module_export_names(module) {
        return names;
    }
    let root = project_root_from(current_path);
    let installed = installed_module_exports_names(module, &root);
    if !installed.is_empty() {
        return installed;
    }

    match module {
        "math" => vec!["sin", "cos", "tan", "sqrt", "abs", "floor", "ceil"],
        "string" => vec!["upper", "lower", "trim", "split", "contains", "starts_with", "ends_with", "replace", "slice", "substring"],
        "random" => vec!["random"],
        "system" => vec!["run", "open", "alert", "confirm", "notify"],
        "time" => vec!["now"],
        "path" => vec!["join", "basename", "dirname", "extension", "is_absolute"],
        "test" => vec!["assert", "assert_equal", "assert_raises"],
        _ => Vec::new(),
    }
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

fn project_root_from(current_path: Option<&Path>) -> PathBuf {
    current_path
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn is_valid_module(module: &str, current_path: Option<&Path>) -> bool {
    if module.starts_with("./") || module.starts_with("../") {
        // Local file imports can be validated statically when we know the
        // directory of the file being checked.
        if let Some(path) = resolve_local_module_path(module, current_path) {
            return path.is_file();
        }
        return false;
    }
    if matches!(module, "math" | "string" | "random" | "system" | "time") {
        return true;
    }
    // Plain module names may also resolve to installed packages, standard-library
    // source files (e.g. `list.period`), or their `.periodi` interfaces.
    let root = project_root_from(current_path);
    crate::package_manager::package_path_in(module, &root).is_some()
        || find_stdlib_module(module).is_some()
        || find_stdlib_interface(module).is_some()
}

pub(crate) fn find_stdlib_module(module: &str) -> Option<PathBuf> {
    let file = format!("{}.period", module);
    for loc in stdlib_locations() {
        let path = loc.join(&file);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

pub(crate) fn find_stdlib_interface(module: &str) -> Option<PathBuf> {
    let file = format!("{}.periodi", module);
    for loc in stdlib_locations() {
        let path = loc.join(&file);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn stdlib_locations() -> Vec<PathBuf> {
    let mut locs = Vec::new();
    if let Ok(v) = env::var("PERIOD_STDLIB") {
        locs.push(PathBuf::from(v));
    }
    if let Ok(exe) = env::current_exe()
        && let Some(parent) = exe.parent() {
            locs.push(parent.join("stdlib"));
            // Development layout: binary next to a `period` project directory.
            locs.push(parent.join("period").join("stdlib"));
            // FHS-style install layout (e.g. /usr/local/bin/period -> /usr/local/share/period/stdlib)
            if let Some(grandparent) = parent.parent() {
                locs.push(grandparent.join("share").join("period").join("stdlib"));
            }
            // Rust cargo development layout: binary is at period/target/<profile>/period,
            // stdlib is at the repository root or under period/stdlib.
            if parent.file_name().map(|n| n == "debug" || n == "release").unwrap_or(false)
                && let Some(repo) = parent.parent().and_then(|p| p.parent()).and_then(|p| p.parent())
            {
                locs.push(repo.join("stdlib"));
                locs.push(repo.join("period").join("stdlib"));
            }
        }
    if let Ok(cwd) = env::current_dir() {
        locs.push(cwd.join("stdlib"));
        // Development layout: run from the repo root while stdlib lives under `period/`.
        locs.push(cwd.join("period").join("stdlib"));
    }
    locs
}

fn exports_from_program(program: &Program) -> Vec<String> {
    let mut names = Vec::new();
    let mut explicit_exports: Vec<String> = Vec::new();
    let mut has_export = false;
    for stmt in &program.statements {
        match stmt {
            Stmt::Define { name, .. } => names.push(name.clone()),
            Stmt::Class { name, .. } => names.push(name.clone()),
            Stmt::Let { name, .. } => names.push(name.clone()),
            Stmt::Read { name, .. } => names.push(name.clone()),
            Stmt::Export(exported) => {
                has_export = true;
                explicit_exports.extend(exported.iter().cloned());
            }
            _ => {}
        }
    }
    if has_export { explicit_exports } else { names }
}

fn stdlib_module_export_names(module: &str) -> Option<Vec<String>> {
    let path = find_stdlib_module(module).or_else(|| find_stdlib_interface(module))?;
    let source = fs::read_to_string(&path).ok()?;
    let program = try_parse(&source).ok()?;
    Some(exports_from_program(&program))
}

/// Parse source text into a Program.
pub(crate) fn try_parse(source: &str) -> Result<Program, Vec<String>> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token().map_err(|e| vec![e])?;
        let eof = matches!(t.kind, TokenKind::Eof);
        tokens.push(t);
        if eof {
            break;
        }
    }
    Parser::new(tokens).parse_program()
}

/// Resolve a relative module path (e.g. `./helper` or `../utils/helper`) to a
/// file path starting from the directory containing `current_path`.
fn resolve_local_module_path(module: &str, current_path: Option<&Path>) -> Option<PathBuf> {
    let current = current_path?;
    let dir = if current.is_file() {
        current.parent().unwrap_or(current)
    } else {
        current
    };

    Some(dir.join(module).with_extension("period"))
}

/// Collect top-level exported names from a local `.period` file.
/// Falls back to an empty list if the file cannot be read or parsed.
fn local_module_exports_names(module: &str, current_path: Option<&Path>) -> Vec<String> {
    let path = match resolve_local_module_path(module, current_path) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let source = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let program = match try_parse(&source) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    exports_from_program(&program)
}

fn installed_module_exports_names(module: &str, root: &Path) -> Vec<String> {
    let path = match crate::package_manager::package_path_in(module, root) {
        Some(p) => root.join(p),
        None => return Vec::new(),
    };
    let source = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let program = match try_parse(&source) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    exports_from_program(&program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdlib_source_module_is_valid() {
        assert!(is_valid_module("list", None));
        assert!(is_valid_module("text", None));
    }
}

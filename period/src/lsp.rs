use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
    CompletionResponse, Diagnostic, DiagnosticSeverity, Hover, HoverContents, HoverOptions,
    HoverParams, HoverProviderCapability, MarkupContent, MarkupKind,
    Position, PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentContentChangeEvent,
    TextDocumentItem, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};

use crate::ast::*;
use crate::lexer::{Lexer, Token, TokenKind};
use crate::parser::Parser;

pub fn run() -> Result<(), Box<dyn Error>> {
    std::panic::set_hook(Box::new(|_| {}));
    let (connection, io_threads) = Connection::stdio();
    let server_capabilities = serde_json::to_value(&ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Options(HoverOptions {
            work_done_progress_options: Default::default(),
        })),
        completion_provider: Some(CompletionOptions {
            resolve_provider: Some(false),
            trigger_characters: Some(vec![".".to_string(), " ".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    })?;

    let _initialization_params = connection.initialize(server_capabilities)?;

    let documents: Arc<Mutex<HashMap<Url, String>>> = Arc::new(Mutex::new(HashMap::new()));

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }
                handle_request(req, &connection, &documents)?;
            }
            Message::Notification(not) => {
                if let Some(uri) = handle_notification(not, &documents) {
                    publish_diagnostics(&connection, &documents, uri)?;
                }
            }
            _ => {}
        }
    }

    io_threads.join()?;
    Ok(())
}

fn handle_request(
    req: Request,
    connection: &Connection,
    documents: &Arc<Mutex<HashMap<Url, String>>>,
) -> Result<(), Box<dyn Error>> {
    let id = req.id.clone();
    match req.method.as_str() {
        "textDocument/hover" => {
            let params: HoverParams = serde_json::from_value(req.params)?;
            let result = hover(documents, params)?;
            let result_value = serde_json::to_value(result)?;
            connection.sender.send(Message::Response(Response {
                id,
                result: Some(result_value),
                error: None,
            }))?;
        }
        "textDocument/completion" => {
            let params: CompletionParams = serde_json::from_value(req.params)?;
            let result = completion(documents, params)?;
            let result_value = serde_json::to_value(result)?;
            connection.sender.send(Message::Response(Response {
                id,
                result: Some(result_value),
                error: None,
            }))?;
        }
        _ => {}
    }
    Ok(())
}

fn handle_notification(not: Notification, documents: &Arc<Mutex<HashMap<Url, String>>>) -> Option<Url> {
    match not.method.as_str() {
        "textDocument/didOpen" => {
            if let Ok(params) = serde_json::from_value::<DidOpenTextDocumentParams>(not.params) {
                let uri = params.text_document.uri.clone();
                documents.lock().unwrap().insert(params.text_document.uri, params.text_document.text);
                return Some(uri);
            }
        }
        "textDocument/didChange" => {
            if let Ok(params) = serde_json::from_value::<DidChangeTextDocumentParams>(not.params) {
                let uri = params.text_document.uri.clone();
                if let Some(change) = params.content_changes.into_iter().next() {
                    documents.lock().unwrap().insert(params.text_document.uri, change.text);
                }
                return Some(uri);
            }
        }
        "textDocument/didClose" => {
            if let Ok(params) = serde_json::from_value::<DidCloseTextDocumentParams>(not.params) {
                documents.lock().unwrap().remove(&params.text_document.uri);
                return Some(params.text_document.uri);
            }
        }
        _ => {}
    }
    None
}

// Helper structs to deserialize notifications not re-exported by lsp-types conveniently.
#[derive(serde::Deserialize)]
struct DidOpenTextDocumentParams {
    #[serde(rename = "textDocument")]
    text_document: TextDocumentItem,
}

#[derive(serde::Deserialize)]
struct DidChangeTextDocumentParams {
    #[serde(rename = "textDocument")]
    text_document: VersionedTextDocumentIdentifier,
    #[serde(rename = "contentChanges")]
    content_changes: Vec<TextDocumentContentChangeEvent>,
}

#[derive(serde::Deserialize)]
struct VersionedTextDocumentIdentifier {
    uri: Url,
}

#[derive(serde::Deserialize)]
struct DidCloseTextDocumentParams {
    #[serde(rename = "textDocument")]
    text_document: TextDocumentIdentifier,
}

#[derive(serde::Deserialize)]
struct TextDocumentIdentifier {
    uri: Url,
}

#[derive(Clone, Debug)]
struct SymbolInfo {
    name: String,
    detail: String,
    docstring: Option<String>,
    kind: CompletionItemKind,
}

fn dedup_symbols(symbols: &mut Vec<SymbolInfo>) {
    let mut seen = std::collections::HashSet::new();
    symbols.retain(|s| seen.insert(s.name.clone()));
}

fn hover(
    documents: &Arc<Mutex<HashMap<Url, String>>>,
    params: HoverParams,
) -> Result<Option<Hover>, Box<dyn Error>> {
    let uri = params.text_document_position_params.text_document.uri;
    let pos = params.text_document_position_params.position;
    let text = match documents.lock().unwrap().get(&uri) {
        Some(t) => t.clone(),
        None => return Ok(None),
    };

    let tokens = match lex_tokens(&text) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let token = match find_token(&tokens, pos) {
        Some(t) => t,
        None => return Ok(None),
    };

    if let Some(doc) = keyword_doc(&token.kind) {
        return Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: doc.to_string(),
            }),
            range: None,
        }));
    }

    let name = match &token.kind {
        TokenKind::Ident(n) => n.clone(),
        _ => return Ok(None),
    };
    eprintln!("hover name={} at {}:{}", name, pos.line, pos.character);

    let program = match try_parse(&text).ok() {
        Some(p) => p,
        None => return Ok(None),
    };
    let symbols = index_program_all(&program);
    let builtins = all_builtins();

    let mut matches: Vec<&SymbolInfo> = symbols.iter().filter(|s| s.name == name).collect();
    if matches.is_empty() {
        matches = builtins.iter().filter(|s| s.name == name).collect();
    }
    if matches.is_empty() {
        return Ok(None);
    }

    let mut md = String::new();
    for (i, sym) in matches.iter().enumerate() {
        if i > 0 {
            md.push_str("\n\n---\n\n");
        }
        let code = if sym.kind == CompletionItemKind::MODULE {
            format!("import {}.", sym.name)
        } else {
            sym.detail.clone()
        };
        md.push_str(&format!("```period\n{}\n```", code));
        if let Some(doc) = &sym.docstring {
            md.push_str(&format!("\n\n{}", doc));
        }
    }

    Ok(Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: md,
        }),
        range: None,
    }))
}

fn keyword_doc(kind: &TokenKind) -> Option<&'static str> {
    Some(match kind {
        TokenKind::Show => "```period\nshow <expression>.\n```\n\nPrint the value of an expression.",
        TokenKind::Let => "```period\nlet <name> be <expression>.\n```\n\nDeclare a new variable.",
        TokenKind::Set => "```period\nset <target> to <expression>.\n```\n\nAssign a new value to an existing variable, index, or property.",
        TokenKind::If => "```period\nif <condition>, then:\n    ...\n```\n\nConditionally execute a block.",
        TokenKind::Then => "Part of an `if` statement: `if <condition>, then:`.",
        TokenKind::Otherwise => "```period\notherwise:\n    ...\n```\n\nElse branch of an `if` statement.",
        TokenKind::While => "```period\nwhile <condition> repeat:\n    ...\n```\n\nLoop while a condition is true.",
        TokenKind::Repeat => "Part of a `while` loop: `while <condition> repeat:`.",
        TokenKind::For => "```period\nfor <var> in <iterable> repeat:\n    ...\n```\n\nIterate over a list or range.",
        TokenKind::In => "Part of a `for` loop: `for <var> in <iterable>`.",
        TokenKind::Define => "```period\ndefine <name> with <params> returns <type>:\n    ...\n```\n\nDefine a function.",
        TokenKind::With => "Used in function definitions and calls: `define <name> with <params>.` / `<name> with <args>.`",
        TokenKind::Returns => "Used in function signatures: `define <name> with <params> returns <type>:`.",
        TokenKind::Return => "```period\nreturn <expression>.\n```\n\nReturn a value from a function.",
        TokenKind::Class => "```period\nclass <Name>:\n    ...\n```\n\nDefine a class.",
        TokenKind::New => "```period\nnew <Class>.\n```\n\nCreate a new instance of a class.",
        TokenKind::Import => "```period\nimport <module>.\n```\n\nImport a built-in or standard-library module. Use a leading dot (e.g. `.helper`) for local files.",
        TokenKind::From => "```period\n<name> from <module>.\n```\n\nUse or import a specific name from a module.",
        TokenKind::Tell => "```period\ntell <object> to <method> with <args>.\n```\n\nSend a message to an object.",
        _ => return None,
    })
}

fn completion(
    documents: &Arc<Mutex<HashMap<Url, String>>>,
    params: CompletionParams,
) -> Result<Option<CompletionResponse>, Box<dyn Error>> {
    let uri = params.text_document_position.text_document.uri;
    let text = match documents.lock().unwrap().get(&uri) {
        Some(t) => t.clone(),
        None => return Ok(None),
    };

    let line_text = text.lines().nth(params.text_document_position.position.line as usize).unwrap_or("");

    // An import statement ending with '.' is complete; don't offer completions after it,
    // otherwise pressing Enter would insert the first exported name (e.g. 'abs').
    let trimmed = line_text.trim();
    if trimmed.starts_with("import") && trimmed.ends_with('.') {
        return Ok(Some(CompletionResponse::Array(Vec::new())));
    }

    let mut symbols = Vec::new();
    symbols.extend(keyword_completions());
    symbols.extend(all_builtins());

    // If the file parses, also include user-defined symbols and imports.
    if let Some(program) = try_parse(&text).ok() {
        symbols.extend(index_program(&program));
    }
    dedup_symbols(&mut symbols);

    // If the line contains "<name> from <module>", filter to that module's exports.
    let module_hint = module_from_line(line_text);
    if let Some(module) = module_hint {
        symbols.retain(|s| s.detail.starts_with(&format!("{}::", module)));
    }

    let items: Vec<CompletionItem> = symbols
        .into_iter()
        .map(|s| CompletionItem {
            label: s.name.clone(),
            kind: Some(s.kind),
            detail: Some(s.detail.clone()),
            documentation: s.docstring.map(|d| lsp_types::Documentation::String(d)),
            ..Default::default()
        })
        .collect();

    Ok(Some(CompletionResponse::Array(items)))
}

fn lex_tokens(source: &str) -> Result<Vec<Token>, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let t = lexer.next_token();
            let eof = matches!(t.kind, TokenKind::Eof);
            tokens.push(t);
            if eof { break; }
        }
        tokens
    })) {
        Ok(t) => Ok(t),
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "lex error".to_string()
            };
            Err(msg)
        }
    }
}

fn try_parse(source: &str) -> Result<Program, Diagnostic> {
    let tokens = match lex_tokens(source) {
        Ok(t) => t,
        Err(msg) => return Err(parse_error_to_diagnostic(&msg)),
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Parser::new(tokens).parse_program()
    })) {
        Ok(p) => Ok(p),
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "parse error".to_string()
            };
            Err(parse_error_to_diagnostic(&msg))
        }
    }
}

fn parse_error_to_diagnostic(msg: &str) -> Diagnostic {
    // Expected formats: "lexer error at L:C: ..." or "parse error at L:C: ..."
    let prefix = if msg.starts_with("lexer error at ") {
        Some("lexer error at ".len())
    } else if msg.starts_with("parse error at ") {
        Some("parse error at ".len())
    } else {
        None
    };
    let (line, col, message) = if let Some(start) = prefix {
        let rest = &msg[start..];
        let mut parts = rest.splitn(2, ": ");
        let loc = parts.next().unwrap_or("1:1");
        let mut loc_parts = loc.splitn(2, ':');
        let line: u32 = loc_parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
        let col: u32 = loc_parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
        (line, col, parts.next().unwrap_or(msg).to_string())
    } else {
        (1, 1, msg.to_string())
    };

    // For invalid-keyword errors, underline the whole keyword. The lexer reports
    // the column just past the token, so the range spans col-len..col.
    let (start_char, end_char) = if let Some(kw_start) = message.find("keyword '") {
        let after = &message[kw_start + "keyword '".len()..];
        if let Some(end_quote) = after.find('\'') {
            let word = &after[..end_quote];
            let word_len = word.chars().count() as u32;
            let end = col.saturating_sub(1);
            let start = end.saturating_sub(word_len);
            (start, end)
        } else {
            (col.saturating_sub(1), col)
        }
    } else {
        (col.saturating_sub(1), col)
    };

    Diagnostic {
        range: Range {
            start: Position { line: line.saturating_sub(1), character: start_char },
            end: Position { line: line.saturating_sub(1), character: end_char },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("period".to_string()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

fn publish_diagnostics(
    connection: &Connection,
    documents: &Arc<Mutex<HashMap<Url, String>>>,
    uri: Url,
) -> Result<(), Box<dyn Error>> {
    let text = match documents.lock().unwrap().get(&uri) {
        Some(t) => t.clone(),
        None => return Ok(()),
    };

    let mut diagnostics = Vec::new();
    match try_parse(&text) {
        Ok(program) => diagnostics.extend(check_program(&program)),
        Err(d) => diagnostics.push(d),
    }

    connection.sender.send(Message::Notification(Notification::new(
        "textDocument/publishDiagnostics".to_string(),
        serde_json::to_value(PublishDiagnosticsParams {
            uri,
            diagnostics,
            version: None,
        })?,
    )))?;
    Ok(())
}

fn token_len(kind: &TokenKind) -> u32 {
    match kind {
        TokenKind::Ident(s) => s.len() as u32,
        TokenKind::String(s) => s.len() as u32,
        TokenKind::Number(n) => format!("{}", n).len() as u32,
        TokenKind::Bool(b) => if *b { 4 } else { 5 },
        TokenKind::Nothing => 7,
        TokenKind::Let => 3,
        TokenKind::Set => 3,
        TokenKind::Show => 4,
        TokenKind::If => 2,
        TokenKind::Then => 4,
        TokenKind::Otherwise => 9,
        TokenKind::While => 5,
        TokenKind::Repeat => 6,
        TokenKind::For => 3,
        TokenKind::In => 2,
        TokenKind::Define => 6,
        TokenKind::With => 4,
        TokenKind::Return => 6,
        TokenKind::Be => 2,
        TokenKind::To => 2,
        TokenKind::And => 3,
        TokenKind::Or => 2,
        TokenKind::Not => 3,
        TokenKind::Class => 5,
        TokenKind::Init => 4,
        TokenKind::New => 3,
        TokenKind::Tell => 4,
        TokenKind::The => 3,
        TokenKind::Of => 2,
        TokenKind::Import => 6,
        TokenKind::From => 4,
        TokenKind::Returns => 7,
        TokenKind::Ellipsis => 3,
        TokenKind::Comma | TokenKind::Dot | TokenKind::Colon
        | TokenKind::LParen | TokenKind::RParen | TokenKind::LBracket | TokenKind::RBracket
        | TokenKind::LBrace | TokenKind::RBrace | TokenKind::Plus | TokenKind::Minus
        | TokenKind::Star | TokenKind::Slash | TokenKind::Percent | TokenKind::Power => 1,
        TokenKind::EqEq | TokenKind::NotEq | TokenKind::LessEq | TokenKind::GreaterEq => 2,
        TokenKind::Less | TokenKind::Greater => 1,
        TokenKind::Indent | TokenKind::Dedent | TokenKind::Newline | TokenKind::Eof => 0,
    }
}

fn find_token(tokens: &[crate::lexer::Token], pos: lsp_types::Position) -> Option<crate::lexer::Token> {
    for token in tokens {
        let line = (token.span.line as u32).saturating_sub(1);
        if line != pos.line {
            continue;
        }
        let len = token_len(&token.kind);
        // span.col is the 1-based column *after* the token.
        let end_col = (token.span.col as u32).saturating_sub(1);
        let start_col = end_col.saturating_sub(len);
        if pos.character >= start_col && pos.character < end_col {
            return Some(token.clone());
        }
    }
    None
}

fn format_params(params: &[(String, Option<String>)]) -> String {
    params
        .iter()
        .map(|(n, t)| match t {
            Some(ty) => format!("<{}: {}>", n, ty),
            None => format!("<{}>", n),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn function_detail(name: &str, params: &[(String, Option<String>)], return_type: &Option<String>) -> String {
    let param_str = format_params(params);
    let ret = return_type.clone().unwrap_or_else(|| "unknown".to_string());
    if params.is_empty() {
        format!("{} -> {}", name, ret)
    } else {
        format!("{} with {} -> {}", name, param_str, ret)
    }
}

fn init_detail(name: &str, params: &[(String, Option<String>)]) -> String {
    let param_str = format_params(params);
    if params.is_empty() {
        name.to_string()
    } else {
        format!("{} with {}", name, param_str)
    }
}

fn index_program(program: &Program) -> Vec<SymbolInfo> {
    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    let mut func_returns: HashMap<String, String> = HashMap::new();

    // First pass: collect function return types.
    for stmt in &program.statements {
        if let Stmt::Define { name, return_type, .. } = stmt {
            if let Some(ret) = return_type {
                func_returns.insert(name.clone(), ret.clone());
            }
        }
        for method in class_methods(stmt) {
            if let Stmt::Define { name, return_type, .. } = method {
                if let Some(ret) = return_type {
                    func_returns.insert(name.clone(), ret.clone());
                }
            }
        }
    }

    for stmt in &program.statements {
        match stmt {
            Stmt::Let { name, value } => {
                let typ = infer_expr_with_funcs(value, &func_returns);
                symbols.push(SymbolInfo {
                    name: name.clone(),
                    detail: format!("{}: {}", name, typ),
                    docstring: None,
                    kind: CompletionItemKind::VARIABLE,
                });
            }
            Stmt::Define { name, params, return_type, docstring, .. } => {
                symbols.push(SymbolInfo {
                    name: name.clone(),
                    detail: function_detail(name, params, return_type),
                    docstring: docstring.clone(),
                    kind: CompletionItemKind::FUNCTION,
                });
            }
            Stmt::Class { name, init, methods, docstring } => {
                symbols.push(SymbolInfo {
                    name: name.clone(),
                    detail: format!("class {}", name),
                    docstring: docstring.clone(),
                    kind: CompletionItemKind::CLASS,
                });
                if let Some(init) = init {
                    symbols.push(SymbolInfo {
                        name: format!("{}.__init__", name),
                        detail: init_detail(name, &init.params),
                        docstring: init.docstring.clone(),
                        kind: CompletionItemKind::CONSTRUCTOR,
                    });
                }
                for m in methods {
                    if let Stmt::Define { name: mname, params, return_type, docstring, .. } = m {
                        symbols.push(SymbolInfo {
                            name: mname.clone(),
                            detail: function_detail(mname, params, return_type),
                            docstring: docstring.clone(),
                            kind: CompletionItemKind::METHOD,
                        });
                    }
                }
            }
            Stmt::Import(paths) => {
                for path in paths {
                    imports.push(path.clone());
                    let mut exports = module_exports(path);
                    let export_names: Vec<String> = exports.iter().map(|e| e.name.clone()).collect();
                    symbols.push(SymbolInfo {
                        name: path.clone(),
                        detail: format!("module {}", path),
                        docstring: Some(format!("Built-in module `{}`. Exports: {}.", path, export_names.join(", "))),
                        kind: CompletionItemKind::MODULE,
                    });
                    symbols.append(&mut exports);
                }
            }
            _ => {}
        }
    }
    dedup_symbols(&mut symbols);
    symbols
}

fn class_methods(stmt: &Stmt) -> &[Stmt] {
    match stmt {
        Stmt::Class { methods, .. } => methods,
        _ => &[],
    }
}

fn index_program_all(program: &Program) -> Vec<SymbolInfo> {
    let mut symbols = Vec::new();
    let mut func_returns: HashMap<String, String> = HashMap::new();

    // First pass: collect function return types for inference.
    for stmt in &program.statements {
        if let Stmt::Define { name, return_type, .. } = stmt {
            if let Some(ret) = return_type {
                func_returns.insert(name.clone(), ret.clone());
            }
        }
        for method in class_methods(stmt) {
            if let Stmt::Define { name, return_type, .. } = method {
                if let Some(ret) = return_type {
                    func_returns.insert(name.clone(), ret.clone());
                }
            }
        }
    }

    for stmt in &program.statements {
        collect_symbols(stmt, &func_returns, &mut symbols);
    }
    dedup_symbols(&mut symbols);
    symbols
}

fn collect_symbols(stmt: &Stmt, func_returns: &HashMap<String, String>, symbols: &mut Vec<SymbolInfo>) {
    match stmt {
        Stmt::Let { name, value } => {
            let typ = infer_expr_with_funcs(value, func_returns);
            symbols.push(SymbolInfo {
                name: name.clone(),
                detail: format!("{}: {}", name, typ),
                docstring: None,
                kind: CompletionItemKind::VARIABLE,
            });
        }
        Stmt::Define { name, params, return_type, docstring, body } => {
            symbols.push(SymbolInfo {
                name: name.clone(),
                detail: function_detail(name, params, return_type),
                docstring: docstring.clone(),
                kind: CompletionItemKind::FUNCTION,
            });
            for s in body { collect_symbols(s, func_returns, symbols); }
        }
        Stmt::Class { name, init, methods, docstring } => {
            symbols.push(SymbolInfo {
                name: name.clone(),
                detail: format!("class {}", name),
                docstring: docstring.clone(),
                kind: CompletionItemKind::CLASS,
            });
            if let Some(init) = init {
                symbols.push(SymbolInfo {
                    name: format!("{}.__init__", name),
                    detail: init_detail(name, &init.params),
                    docstring: init.docstring.clone(),
                    kind: CompletionItemKind::CONSTRUCTOR,
                });
                for s in &init.body { collect_symbols(s, func_returns, symbols); }
            }
            for m in methods { collect_symbols(m, func_returns, symbols); }
        }
        Stmt::Import(paths) => {
            for path in paths {
                let mut exports = module_exports(path);
                let export_names: Vec<String> = exports.iter().map(|e| e.name.clone()).collect();
                symbols.push(SymbolInfo {
                    name: path.clone(),
                    detail: format!("module {}", path),
                    docstring: Some(format!("Built-in module `{}`. Exports: {}.", path, export_names.join(", "))),
                    kind: CompletionItemKind::MODULE,
                });
                symbols.append(&mut exports);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch { collect_symbols(s, func_returns, symbols); }
            for s in else_branch { collect_symbols(s, func_returns, symbols); }
        }
        Stmt::While { body, .. } => {
            for s in body { collect_symbols(s, func_returns, symbols); }
        }
        Stmt::For { body, .. } => {
            for s in body { collect_symbols(s, func_returns, symbols); }
        }
        _ => {}
    }
}

fn infer_expr_with_funcs(expr: &Expr, func_returns: &HashMap<String, String>) -> String {
    match expr {
        Expr::Number(_) => "number".to_string(),
        Expr::String(_) => "string".to_string(),
        Expr::Bool(_) => "boolean".to_string(),
        Expr::Nothing | Expr::Ellipsis => "nothing".to_string(),
        Expr::List(_) => "list".to_string(),
        Expr::Dict(_) => "dictionary".to_string(),
        Expr::New { class, .. } => {
            if let Expr::Variable { name, .. } = class.as_ref() {
                format!("instance of {}", name)
            } else {
                "instance".to_string()
            }
        }
        Expr::Binary { .. } => "number".to_string(),
        Expr::Unary { op, .. } => match op {
            UnaryOp::Neg => "number".to_string(),
            UnaryOp::Not => "boolean".to_string(),
        },
        Expr::Call { callee, .. } => {
            if let Expr::Variable { name, .. } = callee.as_ref() {
                if let Some(ret) = func_returns.get(name) {
                    ret.clone()
                } else {
                    function_return_type(name)
                }
            } else {
                "unknown".to_string()
            }
        }
        _ => "unknown".to_string(),
    }
}

fn function_return_type(name: &str) -> String {
    match name {
        "length" => "integer",
        "string" => "string",
        "number" => "number",
        "type" => "string",
        "input" => "string",
        "range" => "list",
        "sqrt" | "sin" | "cos" | "tan" | "abs" | "floor" | "ceil" => "number",
        "random" => "number",
        "now" => "number",
        "upper" | "lower" => "string",
        _ => "unknown",
    }
    .to_string()
}

fn stdlib_locations() -> Vec<PathBuf> {
    let mut locs = Vec::new();
    if let Ok(v) = env::var("PERIOD_STDLIB") {
        locs.push(PathBuf::from(v));
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            locs.push(parent.join("stdlib"));
        }
    }
    if let Ok(cwd) = env::current_dir() {
        locs.push(cwd.join("stdlib"));
    }
    locs
}

fn find_stdlib_module(module: &str) -> Option<PathBuf> {
    let file = format!("{}.period", module);
    for loc in stdlib_locations() {
        let path = loc.join(&file);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn find_stdlib_interface(module: &str) -> Option<PathBuf> {
    let file = format!("{}.periodi", module);
    for loc in stdlib_locations() {
        let path = loc.join(&file);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn stdlib_module_exports(module: &str) -> Option<Vec<SymbolInfo>> {
    let path = find_stdlib_module(module).or_else(|| find_stdlib_interface(module))?;
    let source = fs::read_to_string(&path).ok()?;
    let program = try_parse(&source).ok()?;

    let mut func_returns: HashMap<String, String> = HashMap::new();
    for stmt in &program.statements {
        if let Stmt::Define { name, return_type, .. } = stmt {
            if let Some(ret) = return_type {
                func_returns.insert(name.clone(), ret.clone());
            }
        }
    }

    let mut exports = Vec::new();
    for stmt in &program.statements {
        match stmt {
            Stmt::Let { name, value } => {
                let typ = infer_expr_with_funcs(value, &func_returns);
                exports.push(SymbolInfo {
                    name: name.clone(),
                    detail: format!("{}: {}", name, typ),
                    docstring: None,
                    kind: CompletionItemKind::VARIABLE,
                });
            }
            Stmt::Define { name, params, return_type, docstring, .. } => {
                exports.push(SymbolInfo {
                    name: name.clone(),
                    detail: format!("{}::{}", module, function_detail(name, params, return_type)),
                    docstring: docstring.clone(),
                    kind: CompletionItemKind::FUNCTION,
                });
            }
            Stmt::Class { name, init, methods, docstring } => {
                exports.push(SymbolInfo {
                    name: name.clone(),
                    detail: format!("{}::class {}", module, name),
                    docstring: docstring.clone(),
                    kind: CompletionItemKind::CLASS,
                });
                if let Some(init) = init {
                    exports.push(SymbolInfo {
                        name: format!("{}.__init__", name),
                        detail: format!("{}::{}", module, init_detail(name, &init.params)),
                        docstring: init.docstring.clone(),
                        kind: CompletionItemKind::CONSTRUCTOR,
                    });
                }
                for m in methods {
                    if let Stmt::Define { name: mname, params, return_type, docstring, .. } = m {
                        exports.push(SymbolInfo {
                            name: mname.clone(),
                            detail: format!("{}::{}", module, function_detail(mname, params, return_type)),
                            docstring: docstring.clone(),
                            kind: CompletionItemKind::METHOD,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    Some(exports)
}

fn module_exports(module: &str) -> Vec<SymbolInfo> {
    if let Some(exports) = stdlib_module_exports(module) {
        return exports;
    }

    let exports = match module {
        "math" => vec![
            builtin_fn("sin", "number", "number", "Return the sine of a value."),
            builtin_fn("cos", "number", "number", "Return the cosine of a value."),
            builtin_fn("tan", "number", "number", "Return the tangent of a value."),
            builtin_fn("sqrt", "number", "number", "Return the square root of a value."),
            builtin_fn("abs", "number", "number", "Return the absolute value."),
            builtin_fn("floor", "number", "number", "Round down to the nearest integer."),
            builtin_fn("ceil", "number", "number", "Round up to the nearest integer."),
        ],
        "string" => vec![
            builtin_fn("upper", "string", "string", "Convert a string to uppercase."),
            builtin_fn("lower", "string", "string", "Convert a string to lowercase."),
        ],
        "random" => vec![
            builtin_fn("random", "", "number", "Return a random number between 0 and 1."),
        ],
        "time" => vec![
            builtin_fn("now", "", "number", "Return the current Unix timestamp."),
        ],
        _ => Vec::new(),
    };
    exports
        .into_iter()
        .map(|mut s| {
            s.detail = format!("{}::{}", module, s.detail);
            s
        })
        .collect()
}

fn all_builtins() -> Vec<SymbolInfo> {
    let mut out = Vec::new();
    out.push(builtin_fn("length", "value", "integer", "Return the length of a string or list."));
    out.push(builtin_fn("string", "value", "string", "Convert a value to a string."));
    out.push(builtin_fn("number", "value", "number", "Convert a value to a number."));
    out.push(builtin_fn("type", "value", "string", "Return the type name of a value."));
    out.push(builtin_fn("input", "", "string", "Read a line of input from the user."));
    out.push(builtin_fn("range", "stop", "list", "Return a list of integers from 0 to stop-1."));
    out.extend(module_exports("math"));
    out.extend(module_exports("string"));
    out.extend(module_exports("random"));
    out.extend(module_exports("time"));
    out
}

fn keyword_completions() -> Vec<SymbolInfo> {
    let keywords = [
        ("import", "import <module>."),
        ("from", "<name> from <module>."),
        ("with", "define <name> with <params>."),
        ("define", "define <name> with <params> returns <type>:"),
        ("returns", "Used in function signatures."),
        ("return", "return <expression>."),
        ("class", "class <Name>:"),
        ("new", "new <Class>."),
        ("tell", "tell <object> to <method> with <args>."),
        ("if", "if <condition>, then:"),
        ("then", "Part of an if statement."),
        ("otherwise", "otherwise:"),
        ("while", "while <condition> repeat:"),
        ("repeat", "Part of a while/for statement."),
        ("for", "for <var> in <iterable> repeat:"),
        ("in", "Part of a for statement."),
        ("let", "let <name> be <expression>."),
        ("set", "set <target> to <expression>."),
        ("show", "show <expression>."),
        ("and", "Logical and."),
        ("or", "Logical or."),
        ("not", "Logical not."),
    ];
    keywords
        .iter()
        .map(|(name, detail)| SymbolInfo {
            name: name.to_string(),
            detail: detail.to_string(),
            docstring: None,
            kind: CompletionItemKind::KEYWORD,
        })
        .collect()
}

fn builtin_fn(name: &str, param: &str, ret: &str, doc: &str) -> SymbolInfo {
    let detail = if param.is_empty() {
        format!("{} -> {}", name, ret)
    } else {
        format!("{} with <{}> -> {}", name, param, ret)
    };
    SymbolInfo {
        name: name.to_string(),
        detail,
        docstring: Some(doc.to_string()),
        kind: CompletionItemKind::FUNCTION,
    }
}

fn module_from_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(idx) = trimmed.find(" from ") {
        let rest = &trimmed[idx + 6..];
        let module = rest.split_whitespace().next()?;
        return Some(module.to_string());
    }
    None
}

fn check_program(program: &Program) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut imports: Vec<String> = Vec::new();

    // Pre-collect top-level names so functions/classes/imports can be used before
    // their definition site (needed for recursion and cross-references).
    let mut global: Vec<String> = builtin_globals().iter().map(|s| s.to_string()).collect();
    for stmt in &program.statements {
        match stmt {
            Stmt::Define { name, .. } => global.push(name.clone()),
            Stmt::Class { name, .. } => global.push(name.clone()),
            Stmt::Import(paths) => {
                imports.extend(paths.iter().cloned());
                for path in paths {
                    let exposed = path.rsplit('.').next().unwrap_or(path);
                    global.push(exposed.to_string());
                    global.extend(module_exports_names(path));
                }
            }
            _ => {}
        }
    }

    check_block(&program.statements, &global, &imports, &mut diags);
    diags
}

fn builtin_globals() -> Vec<&'static str> {
    vec!["length", "string", "number", "type", "input", "range"]
}

fn check_block(stmts: &[Stmt], scope: &[String], imports: &[String], diags: &mut Vec<Diagnostic>) {
    let mut local = scope.to_vec();
    for stmt in stmts {
        check_stmt(stmt, &mut local, imports, diags);
    }
}

fn check_stmt(stmt: &Stmt, scope: &mut Vec<String>, imports: &[String], diags: &mut Vec<Diagnostic>) {
    match stmt {
        Stmt::Show(expr) | Stmt::Expr(expr) | Stmt::Return(Some(expr)) => check_expr(expr, scope, imports, diags),
        Stmt::Let { name, value } => {
            check_expr(value, scope, imports, diags);
            scope.push(name.clone());
        }
        Stmt::Set { target, value } => {
            check_assign_target(target, scope, imports, diags);
            check_expr(value, scope, imports, diags);
        }
        Stmt::If { cond, then_branch, else_branch } => {
            check_expr(cond, scope, imports, diags);
            check_block(then_branch, scope, imports, diags);
            check_block(else_branch, scope, imports, diags);
        }
        Stmt::While { cond, body } => {
            check_expr(cond, scope, imports, diags);
            check_block(body, scope, imports, diags);
        }
        Stmt::For { var, iterable, body } => {
            check_expr(iterable, scope, imports, diags);
            let mut for_scope = scope.clone();
            for_scope.push(var.clone());
            check_block(body, &for_scope, imports, diags);
        }
        Stmt::Define { params, body, .. } => {
            let mut func_scope = scope.clone();
            for (p, _) in params { func_scope.push(p.clone()); }
            check_block(body, &func_scope, imports, diags);
        }
        Stmt::Class { init, methods, .. } => {
            if let Some(init) = init {
                let mut init_scope = scope.clone();
                for (p, _) in &init.params { init_scope.push(p.clone()); }
                init_scope.push("this".to_string());
                check_block(&init.body, &init_scope, imports, diags);
            }
            for m in methods {
                if let Stmt::Define { params, body, .. } = m {
                    let mut method_scope = scope.clone();
                    for (p, _) in params { method_scope.push(p.clone()); }
                    method_scope.push("this".to_string());
                    check_block(body, &method_scope, imports, diags);
                }
            }
        }
        _ => {}
    }
}

fn check_assign_target(target: &AssignTarget, scope: &[String], imports: &[String], diags: &mut Vec<Diagnostic>) {
    match target {
        AssignTarget::Variable(_) => {}
        AssignTarget::Index { object, index } => {
            check_expr(object, scope, imports, diags);
            check_expr(index, scope, imports, diags);
        }
        AssignTarget::Property { object, .. } => {
            check_expr(object, scope, imports, diags);
        }
    }
}

fn check_expr(expr: &Expr, scope: &[String], imports: &[String], diags: &mut Vec<Diagnostic>) {
    match expr {
        Expr::Variable { name, span } => {
            if !is_defined(name, scope) {
                diags.push(make_diagnostic(span, name, "undefined variable"));
            }
        }
        Expr::Call { callee, args } => {
            if let Expr::Variable { name, span } = callee.as_ref() {
                if !is_defined(name, scope) {
                    diags.push(make_diagnostic(span, name, "undefined function"));
                }
            } else {
                check_expr(callee, scope, imports, diags);
            }
            for a in args { check_expr(a, scope, imports, diags); }
        }
        Expr::Qualified { name, module } => {
            if imports.contains(module) {
                if !module_exports_names(module).contains(&name.clone()) {
                    // module export missing; no span available in this variant
                }
            }
        }
        Expr::New { class, args } => {
            if let Expr::Variable { name, span } = class.as_ref() {
                if !is_defined(name, scope) {
                    diags.push(make_diagnostic(span, name, "undefined class"));
                }
            } else {
                check_expr(class, scope, imports, diags);
            }
            for a in args { check_expr(a, scope, imports, diags); }
        }
        Expr::Binary { left, right, .. } => {
            check_expr(left, scope, imports, diags);
            check_expr(right, scope, imports, diags);
        }
        Expr::Unary { operand, .. } => check_expr(operand, scope, imports, diags),
        Expr::Index { object, index } => {
            check_expr(object, scope, imports, diags);
            check_expr(index, scope, imports, diags);
        }
        Expr::Property { object, .. } => check_expr(object, scope, imports, diags),
        Expr::Tell { object, args, .. } => {
            check_expr(object, scope, imports, diags);
            for a in args { check_expr(a, scope, imports, diags); }
        }
        Expr::List(elems) => {
            for e in elems { check_expr(e, scope, imports, diags); }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                check_expr(k, scope, imports, diags);
                check_expr(v, scope, imports, diags);
            }
        }
        _ => {}
    }
}

fn is_defined(name: &str, scope: &[String]) -> bool {
    scope.contains(&name.to_string())
}

fn module_exports_names(module: &str) -> Vec<String> {
    module_exports(module).into_iter().map(|s| s.name).collect()
}

fn make_diagnostic(span: &Span, name: &str, kind: &str) -> Diagnostic {
    let line = span.line.saturating_sub(1) as u32;
    let len = name.len() as u32;
    let end_col = span.col.saturating_sub(1) as u32;
    let start_col = end_col.saturating_sub(len);
    Diagnostic {
        range: Range {
            start: Position { line, character: start_col },
            end: Position { line, character: end_col },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("period".to_string()),
        message: format!("{} '{}' (did you forget 'from <module>'?)", kind, name),
        related_information: None,
        tags: None,
        data: None,
    }
}

use std::collections::HashMap;
use std::error::Error;
use std::sync::{Arc, Mutex};

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
    CompletionResponse, Diagnostic, DiagnosticSeverity, GotoDefinitionParams, Hover, HoverContents,
    HoverParams, HoverProviderCapability, Location, MarkupContent, MarkupKind,
    Position, PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentContentChangeEvent,
    TextDocumentItem, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};

use crate::ast::*;
use crate::lexer::{Lexer, Token, TokenKind};
use crate::semantic;
use crate::type_checker;

pub fn run() -> Result<(), Box<dyn Error>> {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };
        eprintln!("period lsp panic: {}", payload);
        if let Some(loc) = info.location() {
            eprintln!("  at {}:{}:{}", loc.file(), loc.line(), loc.column());
        }
    }));
    let (connection, io_threads) = Connection::stdio();
    let server_capabilities = serde_json::to_value(&ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions {
            resolve_provider: Some(false),
            trigger_characters: Some(vec![" ".to_string()]),
            ..Default::default()
        }),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
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
        "textDocument/definition" => {
            let params: GotoDefinitionParams = serde_json::from_value(req.params)?;
            let result = definition(documents, params)?;
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
                let mut docs = documents.lock().unwrap();
                if let Some(current) = docs.get(&params.text_document.uri).cloned() {
                    let updated = apply_content_changes(&current, params.content_changes);
                    docs.insert(params.text_document.uri, updated);
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

fn apply_content_changes(current: &str, changes: Vec<TextDocumentContentChangeEvent>) -> String {
    let mut text = current.to_string();
    for change in changes {
        if let Some(range) = change.range {
            let start = position_to_offset(&text, range.start);
            let end = position_to_offset(&text, range.end);
            if start <= end && end <= text.len() {
                text.replace_range(start..end, &change.text);
            }
        } else {
            text = change.text;
        }
    }
    text
}

fn position_to_offset(text: &str, pos: Position) -> usize {
    let mut offset = 0;
    for (i, line) in text.split('\n').enumerate() {
        if i == pos.line as usize {
            let col = pos.character as usize;
            let line_len = line.chars().count();
            let take = col.min(line_len);
            offset += line.chars().take(take).map(|c| c.len_utf8()).sum::<usize>();
            return offset;
        }
        offset += line.len() + 1; // +1 for '\n'
    }
    text.len()
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
        TokenKind::Interpolated(parts) => {
            if let Some(name) = interpolated_ident_at(parts, &token, pos) {
                name
            } else {
                return Ok(None);
            }
        }
        _ => return Ok(None),
    };

    let program = match try_parse(&text).ok() {
        Some(p) => p,
        None => return Ok(None),
    };
    let symbols = resolve_symbols_at(&program, &name, pos);
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

fn interpolated_ident_at(
    parts: &[crate::lexer::StringPart],
    token: &crate::lexer::Token,
    pos: lsp_types::Position,
) -> Option<String> {
    // span.col is the 1-based start column of the token, matching find_token's logic.
    let start_col = (token.span.col as u32).saturating_sub(1);
    let offset_in_string = pos.character.saturating_sub(start_col);
    // Skip the opening quote.
    let mut current = 1u32;
    for part in parts {
        match part {
            crate::lexer::StringPart::Literal(s) => {
                current += s.len() as u32;
            }
            crate::lexer::StringPart::Expr(s) => {
                let part_len = 2 + s.len() as u32;
                if offset_in_string >= current && offset_in_string < current + part_len {
                    // Don't show hover on the braces themselves.
                    if offset_in_string == current || offset_in_string == current + part_len - 1 {
                        return None;
                    }
                    let offset_in_expr = offset_in_string.saturating_sub(current + 1);
                    let sub_tokens = lex_tokens(s).ok()?;
                    if let Some(sub_token) = find_token(&sub_tokens, Position {
                        line: 0,
                        character: offset_in_expr,
                    })
                        && let TokenKind::Ident(name) = &sub_token.kind {
                            return Some(name.clone());
                        }
                }
                current += part_len;
            }
        }
    }
    None
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
        TokenKind::Import => "```period\nimport <module>.\n```\n\nImport a built-in or standard-library module. For local files use a POSIX-style relative path (e.g. `import ./helper.` or `import ../utils/helper.`).",
        TokenKind::From => "```period\n<name> from <module>.\n```\n\nUse or import a specific name from a module.",
        TokenKind::Tell => "```period\ntell <object> to <method> with <args>.\n```\n\nSend a message to an object.",
        TokenKind::Read => "```period\nread <variable> from <path>.\n```\n\nRead the contents of a file into a variable.",
        TokenKind::Write => "```period\nwrite <content> to <path>.\n```\n\nWrite a string to a file.",
        TokenKind::Try => "```period\ntry:\n    ...\ncatch err:\n    ...\n```\n\nRun a block and handle runtime errors in the catch block.",
        TokenKind::Catch => "```period\ncatch <variable>:\n    ...\n```\n\nHandle an error raised in the matching try block.",
        TokenKind::Export => "```period\nexport name1, name2.\n```\n\nMark top-level names as public when this file is imported.",
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
    if let Ok(program) = try_parse(&text) {
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
            documentation: s.docstring.map(lsp_types::Documentation::String),
            ..Default::default()
        })
        .collect();

    Ok(Some(CompletionResponse::Array(items)))
}

fn definition(
    documents: &Arc<Mutex<HashMap<Url, String>>>,
    params: GotoDefinitionParams,
) -> Result<Option<Vec<Location>>, Box<dyn Error>> {
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
    let name = match &token.kind {
        TokenKind::Ident(n) => n.clone(),
        _ => return Ok(None),
    };

    let mut locations = Vec::new();
    for (def_name, span) in collect_local_definitions(&tokens) {
        if def_name == name {
            locations.push(Location {
                uri: uri.clone(),
                range: span_to_range(&span, def_name.len() as u32),
            });
        }
    }

    if locations.is_empty() {
        Ok(None)
    } else {
        Ok(Some(locations))
    }
}

fn span_to_range(span: &Span, len: u32) -> Range {
    let line = (span.line as u32).saturating_sub(1);
    let start_col = (span.col as u32).saturating_sub(1);
    let end_col = start_col + len;
    Range {
        start: Position { line, character: start_col },
        end: Position { line, character: end_col },
    }
}

fn ident_name(kind: &TokenKind) -> Option<String> {
    if let TokenKind::Ident(name) = kind {
        Some(name.clone())
    } else {
        None
    }
}

fn next_ident(start: usize, tokens: &[Token]) -> Option<usize> {
    for (i, t) in tokens.iter().enumerate().skip(start) {
        if matches!(t.kind, TokenKind::Ident(_)) {
            return Some(i);
        }
    }
    None
}

fn prev_significant(start: usize, tokens: &[Token]) -> Option<usize> {
    for i in (0..start).rev() {
        if !matches!(tokens[i].kind, TokenKind::Indent | TokenKind::Dedent | TokenKind::Newline) {
            return Some(i);
        }
    }
    None
}

fn next_significant(start: usize, tokens: &[Token]) -> Option<usize> {
    tokens.iter().enumerate().skip(start).find(|(_, t)| {
        !matches!(t.kind, TokenKind::Indent | TokenKind::Dedent | TokenKind::Newline)
    }).map(|(i, _)| i)
}

fn collect_local_definitions(tokens: &[Token]) -> Vec<(String, Span)> {
    let mut defs = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i].kind {
            TokenKind::Let => {
                // let [type] name be value.  The identifier just before 'be' is the variable.
                if let Some(be_idx) = find_keyword_after(i, tokens, &TokenKind::Be)
                    && let Some(name_idx) = prev_ident_before(be_idx, tokens)
                        && let Some(name) = ident_name(&tokens[name_idx].kind) {
                            defs.push((name, tokens[name_idx].span.clone()));
                        }
            }
            TokenKind::Read => {
                // read name from path.
                if let Some(from_idx) = find_keyword_after(i, tokens, &TokenKind::From)
                    && let Some(name_idx) = prev_ident_before(from_idx, tokens)
                        && let Some(name) = ident_name(&tokens[name_idx].kind) {
                            defs.push((name, tokens[name_idx].span.clone()));
                        }
            }
            TokenKind::Catch => {
                // catch var:
                if let Some(colon_idx) = find_keyword_after(i, tokens, &TokenKind::Colon)
                    && let Some(name_idx) = prev_ident_before(colon_idx, tokens)
                        && let Some(name) = ident_name(&tokens[name_idx].kind) {
                            defs.push((name, tokens[name_idx].span.clone()));
                        }
            }
            TokenKind::For => {
                // for var in iterable
                if let Some(in_idx) = find_keyword_after(i, tokens, &TokenKind::In)
                    && let Some(name_idx) = prev_ident_before(in_idx, tokens)
                        && let Some(name) = ident_name(&tokens[name_idx].kind) {
                            defs.push((name, tokens[name_idx].span.clone()));
                        }
            }
            TokenKind::Define | TokenKind::Class => {
                if let Some(name_idx) = next_ident(i + 1, tokens)
                    && let Some(name) = ident_name(&tokens[name_idx].kind) {
                        defs.push((name, tokens[name_idx].span.clone()));
                    }
            }
            TokenKind::With
                if is_definition_with(i, tokens) => {
                    collect_params(i + 1, tokens, &mut defs);
                }
            _ => {}
        }
        i += 1;
    }
    defs
}

fn find_keyword_after(start: usize, tokens: &[Token], target: &TokenKind) -> Option<usize> {
    tokens.iter().enumerate().skip(start).find(|(_, t)| {
        !matches!(t.kind, TokenKind::Eof) && std::mem::discriminant(&t.kind) == std::mem::discriminant(target)
    }).map(|(i, _)| i)
}

fn prev_ident_before(start: usize, tokens: &[Token]) -> Option<usize> {
    for i in (0..start).rev() {
        if matches!(tokens[i].kind, TokenKind::Ident(_)) {
            return Some(i);
        }
    }
    None
}

fn is_definition_with(idx: usize, tokens: &[Token]) -> bool {
    // init with ...
    if let Some(prev) = prev_significant(idx, tokens) {
        if matches!(tokens[prev].kind, TokenKind::Init) {
            return true;
        }
        // define name with ...
        if let Some(prev2) = prev_significant(prev, tokens)
            && matches!(tokens[prev2].kind, TokenKind::Define) {
                return true;
            }
    }
    false
}

fn collect_params(start: usize, tokens: &[Token], defs: &mut Vec<(String, Span)>) {
    for i in start..tokens.len() {
        match &tokens[i].kind {
            TokenKind::Colon | TokenKind::Returns | TokenKind::Eof => break,
            TokenKind::Ident(name) => {
                if let Some(next) = next_significant(i + 1, tokens) {
                    match tokens[next].kind {
                        TokenKind::Comma | TokenKind::Returns | TokenKind::Colon => {
                            defs.push((name.clone(), tokens[i].span.clone()));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn lex_tokens(source: &str) -> Result<Vec<Token>, String> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token()?;
        let eof = matches!(t.kind, TokenKind::Eof);
        tokens.push(t);
        if eof { break; }
    }
    Ok(tokens)
}

fn try_parse(source: &str) -> Result<Program, Diagnostic> {
    match semantic::try_parse(source) {
        Ok(p) => Ok(p),
        Err(msg) => Err(parse_error_to_diagnostic(&msg)),
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

    // The lexer/parser now report the 1-based start column, so the range spans
    // col-1..col-1+len. For invalid-keyword errors, underline the whole keyword.
    let (start_char, end_char) = if let Some(kw_start) = message.find("keyword '") {
        let after = &message[kw_start + "keyword '".len()..];
        if let Some(end_quote) = after.find('\'') {
            let word = &after[..end_quote];
            let word_len = word.chars().count() as u32;
            let start = col.saturating_sub(1);
            let end = start + word_len;
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
    let current_path = uri.to_file_path().ok().map(|p| p.as_path().to_path_buf());
    match try_parse(&text) {
        Ok(program) => {
            let (errors, warnings) = semantic::program_diagnostics(&program, current_path.as_deref());
            for (span, msg) in errors {
                diagnostics.push(make_diagnostic(&span, quoted_name(&msg), &msg, DiagnosticSeverity::ERROR));
            }
            for (span, msg) in warnings {
                diagnostics.push(make_diagnostic(&span, quoted_name(&msg), &msg, DiagnosticSeverity::WARNING));
            }
            let mut tc = type_checker::TypeChecker::new();
            let (type_errors, type_warnings) = tc.check(&program);
            for (span, msg) in type_warnings {
                diagnostics.push(make_diagnostic(&span, quoted_name(&msg), &msg, DiagnosticSeverity::WARNING));
            }
            for (span, msg) in type_errors {
                diagnostics.push(make_diagnostic(&span, quoted_name(&msg), &msg, DiagnosticSeverity::ERROR));
            }
        }
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
        TokenKind::Interpolated(parts) => {
            let mut len = 2; // opening and closing quotes
            for part in parts {
                match part {
                    crate::lexer::StringPart::Literal(s) => len += s.len(),
                    crate::lexer::StringPart::Expr(s) => len += 2 + s.len(), // braces + expression
                }
            }
            len as u32
        }
        TokenKind::Integer(n) => format!("{}", n).len() as u32,
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
        TokenKind::Read => 5,
        TokenKind::Write => 5,
        TokenKind::Try => 3,
        TokenKind::Catch => 3,
        TokenKind::Export => 6,
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
        // span.col is the 1-based start column of the token.
        let start_col = (token.span.col as u32).saturating_sub(1);
        let end_col = start_col + len;
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
        if let Stmt::Define { name, return_type, .. } = stmt
            && let Some(ret) = return_type {
                func_returns.insert(name.clone(), ret.clone());
            }
        for method in class_methods(stmt) {
            if let Stmt::Define { name, return_type, .. } = method
                && let Some(ret) = return_type {
                    func_returns.insert(name.clone(), ret.clone());
                }
        }
    }

    for stmt in &program.statements {
        match stmt {
            Stmt::Let { name, value, .. } => {
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
            Stmt::Class { name, init, methods, docstring, .. } => {
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
                for (path, _) in paths {
                    imports.push(path.clone());
                    if semantic::is_valid_module(path, None) {
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

fn infer_expr_with_funcs(expr: &Expr, func_returns: &HashMap<String, String>) -> String {
    match expr {
        Expr::Integer(_, _) => "integer".to_string(),
        Expr::Number(_, _) => "number".to_string(),
        Expr::String(_, _) => "string".to_string(),
        Expr::Bool(_, _) => "boolean".to_string(),
        Expr::Nothing(_) | Expr::Ellipsis => "nothing".to_string(),
        Expr::List(_, _) => "list".to_string(),
        Expr::Dict(_, _) => "dictionary".to_string(),
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

/// Resolve symbols visible at `pos` whose name matches `name`, taking lexical
/// scope and source position into account. Top-level functions/classes/imports
/// are treated as hoisted (visible everywhere at the top level); variables and
/// nested definitions are only visible after their definition and within their
/// enclosing scope.
fn resolve_symbols_at(program: &Program, name: &str, pos: Position) -> Vec<SymbolInfo> {
    let mut func_returns: HashMap<String, String> = HashMap::new();
    for stmt in &program.statements {
        if let Stmt::Define { name: n, return_type, .. } = stmt
            && let Some(ret) = return_type {
            func_returns.insert(n.clone(), ret.clone());
        }
        for method in class_methods(stmt) {
            if let Stmt::Define { name: n, return_type, .. } = method
                && let Some(ret) = return_type {
                func_returns.insert(n.clone(), ret.clone());
            }
        }
    }

    let mut result = Vec::new();

    // Top-level functions, classes and imports are hoisted (matching the
    // semantic checker's forward-ref pass), so they are visible regardless of
    // the cursor position within the top-level scope.
    for stmt in &program.statements {
        match stmt {
            Stmt::Define { name: n, params, return_type, docstring, .. } if n == name => {
                result.push(SymbolInfo {
                    name: n.clone(),
                    detail: function_detail(n, params, return_type),
                    docstring: docstring.clone(),
                    kind: CompletionItemKind::FUNCTION,
                });
            }
            Stmt::Class { name: n, init, methods, docstring, .. } if n == name => {
                result.push(SymbolInfo {
                    name: n.clone(),
                    detail: format!("class {}", n),
                    docstring: docstring.clone(),
                    kind: CompletionItemKind::CLASS,
                });
                if let Some(init) = init {
                    let init_name = format!("{}.__init__", n);
                    if init_name == name {
                        result.push(SymbolInfo {
                            name: init_name,
                            detail: init_detail(n, &init.params),
                            docstring: init.docstring.clone(),
                            kind: CompletionItemKind::CONSTRUCTOR,
                        });
                    }
                }
                for m in methods {
                    if let Stmt::Define { name: mname, params, return_type, docstring, .. } = m {
                        if mname == name {
                            result.push(SymbolInfo {
                                name: mname.clone(),
                                detail: function_detail(mname, params, return_type),
                                docstring: docstring.clone(),
                                kind: CompletionItemKind::METHOD,
                            });
                        }
                    }
                }
            }
            Stmt::Import(paths) => {
                for (path, _) in paths {
                    if path == name && semantic::is_valid_module(path, None) {
                        let exports = module_exports(path);
                        let export_names: Vec<String> = exports.iter().map(|e| e.name.clone()).collect();
                        result.push(SymbolInfo {
                            name: path.clone(),
                            detail: format!("module {}", path),
                            docstring: Some(format!("Built-in module `{}`. Exports: {}.", path, export_names.join(", "))),
                            kind: CompletionItemKind::MODULE,
                        });
                    }
                    if semantic::is_valid_module(path, None) {
                        for export in module_exports(path) {
                            if export.name == name {
                                result.push(export);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    resolve_scope_symbols(&program.statements, name, pos, &func_returns, &mut result, true);
    result
}

fn resolve_scope_symbols(
    stmts: &[Stmt],
    name: &str,
    pos: Position,
    func_returns: &HashMap<String, String>,
    result: &mut Vec<SymbolInfo>,
    is_top_level: bool,
) {
    for stmt in stmts {
        let contains = stmt_contains_pos(stmt, pos);
        let before = stmt_is_before_pos(stmt, pos);

        if before {
            add_matching_def(stmt, name, func_returns, result, is_top_level);
        }

        if contains {
            match stmt {
                Stmt::Define { name: _n, params, body, .. } => {
                    for (param_name, param_type) in params {
                        if param_name == name {
                            result.push(SymbolInfo {
                                name: param_name.clone(),
                                detail: format!("{}: {}", param_name, param_type.as_deref().unwrap_or("unknown")),
                                docstring: None,
                                kind: CompletionItemKind::VARIABLE,
                            });
                        }
                    }
                    resolve_scope_symbols(body, name, pos, func_returns, result, false);
                }
                Stmt::Class { init, methods, .. } => {
                    if let Some(init) = init {
                        for (param_name, param_type) in &init.params {
                            if param_name == name {
                                result.push(SymbolInfo {
                                    name: param_name.clone(),
                                    detail: format!("{}: {}", param_name, param_type.as_deref().unwrap_or("unknown")),
                                    docstring: None,
                                    kind: CompletionItemKind::VARIABLE,
                                });
                            }
                        }
                        resolve_scope_symbols(&init.body, name, pos, func_returns, result, false);
                    }
                    for m in methods {
                        if stmt_contains_pos(m, pos) {
                            if let Stmt::Define { name: _n, params, body, .. } = m {
                                for (param_name, param_type) in params {
                                    if param_name == name {
                                        result.push(SymbolInfo {
                                            name: param_name.clone(),
                                            detail: format!("{}: {}", param_name, param_type.as_deref().unwrap_or("unknown")),
                                            docstring: None,
                                            kind: CompletionItemKind::VARIABLE,
                                        });
                                    }
                                }
                                resolve_scope_symbols(body, name, pos, func_returns, result, false);
                            }
                        }
                    }
                }
                Stmt::If { then_branch, else_branch, .. } => {
                    if branch_contains_pos(then_branch, pos) {
                        resolve_scope_symbols(then_branch, name, pos, func_returns, result, false);
                    } else if branch_contains_pos(else_branch, pos) {
                        resolve_scope_symbols(else_branch, name, pos, func_returns, result, false);
                    }
                }
                Stmt::Try { body, catch_var, catch_body, .. } => {
                    if branch_contains_pos(body, pos) {
                        resolve_scope_symbols(body, name, pos, func_returns, result, false);
                    } else if branch_contains_pos(catch_body, pos) {
                        if catch_var == name {
                            result.push(SymbolInfo {
                                name: catch_var.clone(),
                                detail: format!("{}: string", catch_var),
                                docstring: None,
                                kind: CompletionItemKind::VARIABLE,
                            });
                        }
                        resolve_scope_symbols(catch_body, name, pos, func_returns, result, false);
                    }
                }
                Stmt::While { body, .. } => {
                    if branch_contains_pos(body, pos) {
                        resolve_scope_symbols(body, name, pos, func_returns, result, false);
                    }
                }
                Stmt::For { var, body, .. } => {
                    if branch_contains_pos(body, pos) {
                        if var == name {
                            result.push(SymbolInfo {
                                name: var.clone(),
                                detail: format!("{}: list element", var),
                                docstring: None,
                                kind: CompletionItemKind::VARIABLE,
                            });
                        }
                        resolve_scope_symbols(body, name, pos, func_returns, result, false);
                    }
                }
                _ => {}
            }
            break;
        }
    }
}

fn add_matching_def(
    stmt: &Stmt,
    name: &str,
    func_returns: &HashMap<String, String>,
    result: &mut Vec<SymbolInfo>,
    is_top_level: bool,
) {
    match stmt {
        Stmt::Let { name: n, value, .. } if n == name => {
            let typ = infer_expr_with_funcs(value, func_returns);
            result.push(SymbolInfo {
                name: n.clone(),
                detail: format!("{}: {}", n, typ),
                docstring: None,
                kind: CompletionItemKind::VARIABLE,
            });
        }
        Stmt::Read { name: n, .. } if n == name => {
            result.push(SymbolInfo {
                name: n.clone(),
                detail: format!("{}: string", n),
                docstring: None,
                kind: CompletionItemKind::VARIABLE,
            });
        }
        Stmt::Define { name: n, params, return_type, docstring, .. } if n == name && !is_top_level => {
            result.push(SymbolInfo {
                name: n.clone(),
                detail: function_detail(n, params, return_type),
                docstring: docstring.clone(),
                kind: CompletionItemKind::FUNCTION,
            });
        }
        Stmt::Class { name: n, init, methods, docstring, .. } if !is_top_level => {
            if n == name {
                result.push(SymbolInfo {
                    name: n.clone(),
                    detail: format!("class {}", n),
                    docstring: docstring.clone(),
                    kind: CompletionItemKind::CLASS,
                });
            }
            if let Some(init) = init {
                let init_name = format!("{}.__init__", n);
                if init_name == name {
                    result.push(SymbolInfo {
                        name: init_name,
                        detail: init_detail(n, &init.params),
                        docstring: init.docstring.clone(),
                        kind: CompletionItemKind::CONSTRUCTOR,
                    });
                }
            }
            for m in methods {
                if let Stmt::Define { name: mname, params, return_type, docstring, .. } = m {
                    if mname == name {
                        result.push(SymbolInfo {
                            name: mname.clone(),
                            detail: function_detail(mname, params, return_type),
                            docstring: docstring.clone(),
                            kind: CompletionItemKind::METHOD,
                        });
                    }
                }
            }
        }
        Stmt::Import(paths) if !is_top_level => {
            for (path, _) in paths {
                if path == name && semantic::is_valid_module(path, None) {
                    let exports = module_exports(path);
                    let export_names: Vec<String> = exports.iter().map(|e| e.name.clone()).collect();
                    result.push(SymbolInfo {
                        name: path.clone(),
                        detail: format!("module {}", path),
                        docstring: Some(format!("Built-in module `{}`. Exports: {}.", path, export_names.join(", "))),
                        kind: CompletionItemKind::MODULE,
                    });
                }
                if semantic::is_valid_module(path, None) {
                    for export in module_exports(path) {
                        if export.name == name {
                            result.push(export);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn stmt_contains_pos(stmt: &Stmt, pos: Position) -> bool {
    let start = match stmt_start_line(stmt) {
        Some(s) => s,
        None => return false,
    };
    let end = match stmt_end_line(stmt) {
        Some(e) => e,
        None => return false,
    };
    let line = pos.line as usize + 1;
    line >= start && line <= end
}

fn stmt_is_before_pos(stmt: &Stmt, pos: Position) -> bool {
    match stmt_start_line(stmt) {
        Some(start) => pos.line as usize + 1 > start,
        None => false,
    }
}

fn branch_contains_pos(stmts: &[Stmt], pos: Position) -> bool {
    stmts.iter().any(|s| stmt_contains_pos(s, pos))
}

fn stmt_start_line(stmt: &Stmt) -> Option<usize> {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Define { span, .. }
        | Stmt::Class { span, .. } => Some(span.line),
        Stmt::If { cond, .. } => cond.span().map(|s| s.line),
        Stmt::While { cond, .. } => cond.span().map(|s| s.line),
        Stmt::For { iterable, .. } => iterable.span().map(|s| s.line),
        Stmt::Try { body, .. } => body.first().and_then(stmt_start_line),
        Stmt::Show(e) | Stmt::Expr(e) | Stmt::Set { value: e, .. } | Stmt::Write { content: e, .. } | Stmt::Read { path: e, .. } => e.span().map(|s| s.line),
        Stmt::Import(paths) => paths.first().map(|(_, span)| span.line),
        _ => None,
    }
}

fn stmt_end_line(stmt: &Stmt) -> Option<usize> {
    match stmt {
        Stmt::Let { span, .. } | Stmt::Return { span, .. } => Some(span.line),
        Stmt::Define { body, span, .. } => body.last().and_then(stmt_end_line).or(Some(span.line)),
        Stmt::Class { methods, span, .. } => {
            let last_method_end = methods.last().and_then(|m| match m {
                Stmt::Define { body, .. } => body.last().and_then(stmt_end_line),
                _ => None,
            });
            last_method_end.or(Some(span.line))
        }
        Stmt::If { then_branch, else_branch, .. } => {
            let then_end = then_branch.last().and_then(stmt_end_line);
            let else_end = else_branch.last().and_then(stmt_end_line);
            match (then_end, else_end) {
                (Some(t), Some(e)) => Some(t.max(e)),
                (Some(t), None) => Some(t),
                (None, Some(e)) => Some(e),
                (None, None) => None,
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => body.last().and_then(stmt_end_line),
        Stmt::Try { catch_body, .. } => catch_body.last().and_then(stmt_end_line),
        Stmt::Show(e) | Stmt::Expr(e) | Stmt::Set { value: e, .. } | Stmt::Write { content: e, .. } | Stmt::Read { path: e, .. } => e.span().map(|s| s.line),
        Stmt::Import(paths) => paths.last().map(|(_, span)| span.line),
        _ => None,
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

fn stdlib_module_exports(module: &str) -> Option<Vec<SymbolInfo>> {
    let path = semantic::find_stdlib_module(module).or_else(|| semantic::find_stdlib_interface(module))?;
    let source = std::fs::read_to_string(&path).ok()?;
    let program = semantic::try_parse(&source).ok()?;

    let mut func_returns: HashMap<String, String> = HashMap::new();
    for stmt in &program.statements {
        if let Stmt::Define { name, return_type, .. } = stmt
            && let Some(ret) = return_type {
                func_returns.insert(name.clone(), ret.clone());
            }
    }

    let mut exports = Vec::new();
    for stmt in &program.statements {
        match stmt {
            Stmt::Let { name, value, .. } => {
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
            Stmt::Class { name, init, methods, docstring, .. } => {
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
    let mut out = vec![
        builtin_fn("length", "value", "integer", "Return the length of a string or list."),
        builtin_fn("string", "value", "string", "Convert a value to a string."),
        builtin_fn("number", "value", "number", "Convert a value to a number."),
        builtin_fn("type", "value", "string", "Return the type name of a value."),
        builtin_fn("input", "", "string", "Read a line of input from the user."),
        builtin_fn("range", "stop", "range", "Return a lazy range of integers from 0 to stop-1."),
    ];
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
        ("else", "else:"),
        ("while", "while <condition> repeat:"),
        ("repeat", "Part of a while/for statement."),
        ("for", "for <var> in <iterable> repeat:"),
        ("in", "Part of a for statement."),
        ("let", "let <name> be <expression>."),
        ("set", "set <target> to <expression>."),
        ("show", "show <expression>."),
        ("read", "read <variable> from <path>."),
        ("write", "write <content> to <path>."),
        ("try", "try: ... catch error: ..."),
        ("catch", "catch <variable>:"),
        ("export", "export name1, name2."),
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

/// Extract the name inside single quotes from a diagnostic message so the LSP
/// range can underline the offending token.  Falls back to an empty string when
/// no quoted name is present.
fn quoted_name(message: &str) -> &str {
    if let Some(start) = message.find('\'')
        && let Some(end) = message[start + 1..].find('\'') {
            return &message[start + 1..start + 1 + end];
        }
    ""
}

fn make_diagnostic(span: &Span, name: &str, message: &str, severity: DiagnosticSeverity) -> Diagnostic {
    let line = span.line.saturating_sub(1) as u32;
    let len = name.len() as u32;
    let start_col = span.col.saturating_sub(1) as u32;
    let end_col = start_col + len;
    Diagnostic {
        range: Range {
            start: Position { line, character: start_col },
            end: Position { line, character: end_col },
        },
        severity: Some(severity),
        code: None,
        code_description: None,
        source: Some("period".to_string()),
        message: message.to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Span;

    #[test]
    fn span_to_range_covers_token() {
        // Span stores the 1-indexed start column of a token.
        // For "show abcd." the identifier "abcd" starts at column 6.
        let span = Span { line: 1, col: 6 };
        let range = span_to_range(&span, 4);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 5); // 0-indexed start of "abcd"
        assert_eq!(range.end.character, 9);   // just past "abcd"
    }

    #[test]
    fn diagnostic_range_covers_name() {
        // For "show abcd." the undefined variable "abcd" starts at column 6.
        let span = Span { line: 1, col: 6 };
        let diag = make_diagnostic(&span, "abcd", "undefined variable 'abcd'", DiagnosticSeverity::ERROR);
        assert_eq!(diag.range.start.line, 0);
        assert_eq!(diag.range.start.character, 5);
        assert_eq!(diag.range.end.character, 9);
    }

    #[test]
    fn hover_ignores_later_top_level_let() {
        let source = "show a.\nlet a be 1.";
        let program = try_parse(source).unwrap();
        let symbols = resolve_symbols_at(&program, "a", Position { line: 0, character: 5 });
        assert!(symbols.is_empty());
    }

    #[test]
    fn hover_finds_earlier_variable_in_same_scope() {
        let source = "let a be 1.\nshow a.";
        let program = try_parse(source).unwrap();
        let symbols = resolve_symbols_at(&program, "a", Position { line: 1, character: 5 });
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].detail, "a: integer");
    }

    #[test]
    fn hover_does_not_leak_function_local_to_outer_scope() {
        let source = "define f with x:\n    let a be 1.\nshow a.";
        let program = try_parse(source).unwrap();
        let symbols = resolve_symbols_at(&program, "a", Position { line: 2, character: 5 });
        assert!(symbols.is_empty());
    }
}

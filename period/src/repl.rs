use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};

use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Cmd, ConditionalEventHandler, Event, EventContext, EventHandler, Helper, KeyCode, KeyEvent, Modifiers};

use crate::ast;
use crate::interpreter;
use crate::reporting;
use crate::semantic;
use crate::type_checker;

struct ReplState {
    /// Indentation stack. Always starts with 0 (base level). Each block header
    /// pushes the expected body indent; each completed statement or blank line
    /// inside a block pops one level.
    indent_stack: Vec<usize>,
}

impl ReplState {
    fn new() -> Self {
        Self { indent_stack: vec![0] }
    }

    fn current_indent(&self) -> usize {
        *self.indent_stack.last().unwrap()
    }

    fn pop_to(&mut self, indent: usize) {
        while self.indent_stack.len() > 1 && *self.indent_stack.last().unwrap() > indent {
            self.indent_stack.pop();
        }
    }
}

pub fn run_repl() -> Result<(), Box<dyn std::error::Error>> {
    println!("Period REPL. Type 'exit.' or 'quit.' to leave, or Ctrl+C.");

    let mut interp = interpreter::Interpreter::new();
    if let Ok(cwd) = env::current_dir() {
        interp.set_current_path(cwd.clone());
    }

    let mut repl_history: Vec<ast::Stmt> = Vec::new();
    // Diagnostics of the already-accepted history, so each warning is shown
    // only once instead of being re-reported on every subsequent input.
    let mut history_diags: HashMap<(usize, usize, String), usize> = HashMap::new();

    let helper = ReplHelper;
    let mut editor: rustyline::Editor<ReplHelper, rustyline::history::DefaultHistory> =
        rustyline::Editor::new()?;
    editor.set_helper(Some(helper));

    let state = Arc::new(Mutex::new(ReplState::new()));
    editor.bind_sequence(
        KeyEvent(KeyCode::Enter, Modifiers::NONE),
        EventHandler::Conditional(Box::new(EmptyLineHandler {
            state: Arc::clone(&state),
        })),
    );

    let mut session_line_offset = 0usize;
    let mut line_no = 1usize;
    let mut buffer = String::new();

    loop {
        let current_indent = state.lock().unwrap().current_indent();
        let prompt = format!("{:>3} | ", session_line_offset + line_no);
        let initial = " ".repeat(current_indent);
        let line = match editor.readline_with_initial(&prompt, (&initial, "")) {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("^C");
                buffer.clear();
                line_no = 1;
                state.lock().unwrap().indent_stack = vec![0];
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!();
                break;
            }
            Err(e) => return Err(Box::new(e)),
        };

        let trimmed = line.trim();
        let leading = line.chars().take_while(|c| c.is_whitespace()).count();

        {
            let mut stack = state.lock().unwrap();

            // Blank line inside a block dedents one level; at base level it is
            // ignored entirely by the EmptyLineHandler, but handle it here too.
            if trimmed.is_empty() {
                if stack.indent_stack.len() > 1 {
                    stack.indent_stack.pop();
                }
                continue;
            }

            // Adjust the block stack to the current line's indentation.
            stack.pop_to(leading);

            // Block headers push a new indentation level.
            if trimmed.ends_with(':') && !trimmed.starts_with("--") {
                stack.indent_stack.push(leading + 4);
            }
            // Completed statements inside a block pop back to the parent block.
            else if trimmed.ends_with('.') && stack.indent_stack.len() > 1 {
                stack.indent_stack.pop();
            }
        }

        if buffer.is_empty() && (trimmed == "exit." || trimmed == "quit.") {
            break;
        }

        editor.add_history_entry(&line)?;

        if !buffer.is_empty() {
            buffer.push('\n');
        }
        buffer.push_str(&line);

        let buf_trimmed = buffer.trim();
        if buf_trimmed.is_empty() {
            buffer.clear();
            line_no = 1;
            continue;
        }
        if !buf_trimmed.ends_with('.') {
            line_no += 1;
            continue;
        }
        if state.lock().unwrap().indent_stack.len() > 1 {
            // We are still inside an open block; keep reading lines until the
            // block is closed by a dedent or blank line.
            line_no += 1;
            continue;
        }

        // We have a complete statement. Remember how many raw REPL lines it
        // spanned, then reset the per-statement counters.
        let raw_lines = line_no;
        let base_line = session_line_offset;
        session_line_offset += raw_lines;
        line_no = 1;

        let source_for_display = buffer.clone();
        let source_for_parse = normalize_repl_source(&source_for_display);
        let offset_span = |span: &ast::Span| ast::Span {
            line: span.line + base_line,
            col: span.col,
        };

        match crate::parse_source(&source_for_parse) {
            Ok(program) => {
                let current_path = env::current_dir().ok();
                let mut trial_history = repl_history.clone();
                trial_history.extend(program.statements.clone());
                let trial_program = ast::Program { statements: trial_history };
                let mut had_error = false;
                let (sem_errors, sem_warnings) =
                    semantic::program_diagnostics(&trial_program, current_path.as_deref());
                let (type_errors, type_warnings) = if sem_errors.is_empty() {
                    let mut tc = type_checker::TypeChecker::new();
                    tc.check(&trial_program)
                } else {
                    (Vec::new(), Vec::new())
                };

                // Adjust spans to session line numbers.
                let sem_errors: Vec<(ast::Span, String)> = sem_errors
                    .into_iter()
                    .map(|(s, m)| (offset_span(&s), m))
                    .collect();
                let type_errors: Vec<(ast::Span, String)> = type_errors
                    .into_iter()
                    .map(|(s, m)| (offset_span(&s), m))
                    .collect();

                // Only surface warnings that are new relative to the
                // already-accepted history (multiset difference); history
                // never contains errors, so errors are always fresh.
                // Warning deduplication uses relative spans because the
                // semantic/type checker reports them relative to the
                // trial_program, whose first line is always 1.
                let mut seen = history_diags.clone();
                let mut fresh = |diags: &[(ast::Span, String)]| -> Vec<(ast::Span, String)> {
                    let mut out = Vec::new();
                    for (span, msg) in diags {
                        let key = (span.line, span.col, msg.clone());
                        if let Some(n) = seen.get_mut(&key)
                            && *n > 0
                        {
                            *n -= 1;
                            continue;
                        }
                        out.push((span.clone(), msg.clone()));
                    }
                    out
                };
                for (span, msg) in fresh(&sem_warnings) {
                    reporting::report_source_warning("<repl>", &source_for_display, &offset_span(&span), &msg);
                }
                for (span, msg) in sem_errors {
                    reporting::report_source_error("<repl>", &source_for_display, &span, &msg);
                    had_error = true;
                }
                if !had_error {
                    for (span, msg) in fresh(&type_warnings) {
                        reporting::report_source_warning("<repl>", &source_for_display, &offset_span(&span), &msg);
                    }
                    for (span, msg) in type_errors {
                        reporting::report_source_error("<repl>", &source_for_display, &span, &msg);
                        had_error = true;
                    }
                }
                if !had_error {
                    repl_history.extend(program.statements.clone());
                    history_diags.clear();
                    for (span, msg) in sem_warnings.into_iter().chain(type_warnings) {
                        *history_diags.entry((span.line, span.col, msg)).or_insert(0) += 1;
                    }
                    // Single execution path: compile and run on the bytecode VM.
                    if let Err(ctrl) = interp.interpret(&program, true) {
                        match ctrl {
                            interpreter::Control::RuntimeError(msg, span) => {
                                reporting::report_runtime_error(
                                    "<repl>",
                                    &source_for_display,
                                    &msg,
                                    Some(&offset_span(&span)),
                                );
                            }
                            interpreter::Control::Error(msg) => {
                                reporting::report_runtime_error("<repl>", &source_for_display, &msg, None);
                            }
                            _ => eprintln!("runtime error: {:?}", ctrl),
                        }
                    }
                }
            }
            Err(errors) => {
                for msg in errors {
                    if let Some((line, col, reason)) = parse_error_line_col(&msg) {
                        reporting::report_source_error(
                            "<repl>",
                            &source_for_display,
                            &offset_span(&ast::Span { line, col }),
                            &reason,
                        );
                    } else {
                        eprintln!("parse error: {}", msg);
                    }
                }
            }
        }
        buffer.clear();
    }

    Ok(())
}

struct ReplHelper;

impl Highlighter for ReplHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Owned(highlight_period_line(line))
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Owned(format!("\x1b[90m{}\x1b[0m", prompt))
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Borrowed(hint)
    }

    fn highlight_candidate<'c>(
        &self,
        candidate: &'c str,
        _completion: rustyline::CompletionType,
    ) -> Cow<'c, str> {
        Cow::Borrowed(candidate)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _forced: bool) -> bool {
        true
    }
}

impl Validator for ReplHelper {
    fn validate(&self, _ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        // Multiline input is handled manually in the read loop so that each
        // continuation line can have its own numbered prompt and auto-indent.
        Ok(ValidationResult::Valid(None))
    }
}

struct EmptyLineHandler {
    state: Arc<Mutex<ReplState>>,
}

impl ConditionalEventHandler for EmptyLineHandler {
    fn handle(
        &self,
        evt: &Event,
        _n: usize,
        _positive: bool,
        ctx: &EventContext,
    ) -> Option<Cmd> {
        if let Event::KeySeq(keys) = evt {
            if keys.len() == 1
                && keys[0].0 == KeyCode::Enter
                && ctx.line().trim().is_empty()
            {
                // At the base level, an empty Enter does nothing and the cursor
                // stays on the same line. Inside a block, let it through so the
                // main loop can dedent.
                if self.state.lock().unwrap().current_indent() == 0 {
                    return Some(Cmd::Noop);
                }
            }
        }
        None
    }
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        _line: &str,
        _pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        Ok((0, Vec::new()))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;
}

impl Helper for ReplHelper {}

const KEYWORDS: &[&str] = &[
    "let", "set", "show", "if", "then", "otherwise", "else", "while", "repeat", "for", "in",
    "define", "with", "return", "be", "to", "and", "or", "not", "class", "init", "new", "tell",
    "the", "of", "import", "from", "returns", "read", "write", "try", "catch", "export",
];

fn highlight_period_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 64);
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        // Whitespace
        if c.is_whitespace() {
            out.push(c);
            i += 1;
            continue;
        }

        // Comments: `--` to end of line
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            out.push_str("\x1b[2;32m");
            while i < chars.len() {
                out.push(chars[i]);
                i += 1;
            }
            out.push_str("\x1b[0m");
            break;
        }

        // String literals
        if c == '"' || c == '\'' {
            let quote = c;
            out.push_str("\x1b[33m");
            out.push(c);
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                out.push(ch);
                if ch == '\\' && i + 1 < chars.len() {
                    i += 1;
                    out.push(chars[i]);
                } else if ch == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push_str("\x1b[0m");
            continue;
        }

        // Numbers (integer or float)
        if c.is_ascii_digit() || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            out.push_str("\x1b[33m");
            while i < chars.len()
                && (chars[i].is_ascii_digit() || chars[i] == '_' || chars[i] == '.')
            {
                out.push(chars[i]);
                i += 1;
            }
            out.push_str("\x1b[0m");
            continue;
        }

        // Identifiers and keywords
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let lower = word.to_ascii_lowercase();
            if KEYWORDS.contains(&lower.as_str()) {
                out.push_str("\x1b[1;36m");
                out.push_str(&word);
                out.push_str("\x1b[0m");
            } else if lower == "true" || lower == "false" || lower == "nothing" {
                out.push_str("\x1b[1;33m");
                out.push_str(&word);
                out.push_str("\x1b[0m");
            } else {
                out.push_str(&word);
            }
            continue;
        }

        // Operators
        if "+-*/%=<>!".contains(c) {
            out.push_str("\x1b[1;35m");
            out.push(c);
            out.push_str("\x1b[0m");
            i += 1;
            continue;
        }

        // Colon (block introducer)
        if c == ':' {
            out.push_str("\x1b[1;35m");
            out.push(c);
            out.push_str("\x1b[0m");
            i += 1;
            continue;
        }

        // Everything else (punctuation like . , ( ) [ ] { })
        out.push(c);
        i += 1;
    }
    out
}


/// If the last non-empty line of the REPL buffer is a standalone `.`, merge it
/// onto the previous non-empty line. This lets users terminate a statement by
/// typing `.` on its own line, matching the visual flow of the numbered REPL.
fn normalize_repl_source(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    if lines.len() < 2 {
        return raw.to_string();
    }
    let last = lines.last().unwrap().trim();
    if last != "." {
        return raw.to_string();
    }
    for i in (0..lines.len() - 1).rev() {
        if !lines[i].trim().is_empty() {
            let mut result = String::new();
            for j in 0..i {
                result.push_str(lines[j]);
                result.push('\n');
            }
            result.push_str(lines[i].trim_end());
            result.push('.');
            return result;
        }
    }
    raw.to_string()
}

/// Parse a "lexer/parse error at L:C: reason" string. Returns `(line, col, reason)`.
fn parse_error_line_col(msg: &str) -> Option<(usize, usize, String)> {
    let rest = msg
        .strip_prefix("parse error at ")
        .or_else(|| msg.strip_prefix("lexer error at "))?;
    let mut parts = rest.splitn(2, ": ");
    let loc = parts.next()?;
    let mut loc_parts = loc.splitn(2, ':');
    let line: usize = loc_parts.next()?.parse().ok()?;
    let col: usize = loc_parts.next()?.parse().ok()?;
    let reason = parts.next().unwrap_or(msg).to_string();
    Some((line, col, reason))
}

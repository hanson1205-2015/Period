//! Terminal error-reporting helpers shared by the CLI and REPL.

use crate::ast::Span;

fn quoted_token_len(msg: &str) -> usize {
    // Many diagnostics mention the offending token in single quotes,
    // e.g. "undefined variable 'ab'". Underline that whole token.
    if let Some(start) = msg.find('\'') {
        if let Some(end) = msg[start + 1..].find('\'') {
            return msg[start + 1..start + 1 + end].chars().count().max(1);
        }
    }
    1
}

fn source_caret_block(source: &str, span: &Span, underline_len: usize) -> Option<String> {
    let src_line = source.lines().nth(span.line.saturating_sub(1))?;
    let prefix = format!("    {} | ", span.line);
    let indent = prefix.len() + span.col.saturating_sub(1);
    let marker = "^".repeat(underline_len.max(1));
    Some(format!("{}{}\n{}{}", prefix, src_line, " ".repeat(indent), marker))
}

/// Report a source-level error with file location and caret.
pub fn report_source_error(path: &str, source: &str, span: &Span, msg: &str) {
    eprintln!("{}:{}:{}: error: {}", path, span.line, span.col, msg);
    if let Some(block) = source_caret_block(source, span, quoted_token_len(msg)) {
        eprintln!("{}", block);
    }
}

/// Report a source-level warning with file location and caret.
pub fn report_source_warning(path: &str, source: &str, span: &Span, msg: &str) {
    eprintln!("{}:{}:{}: warning: {}", path, span.line, span.col, msg);
    if let Some(block) = source_caret_block(source, span, quoted_token_len(msg)) {
        eprintln!("{}", block);
    }
}

/// Report a runtime error produced by the interpreter.
pub fn report_runtime_error(path: &str, source: &str, msg: &str, span: Option<&Span>) {
    if let Some(span) = span {
        eprintln!("{}:{}:{}: runtime error: {}", path, span.line, span.col, msg);
        if let Some(block) = source_caret_block(source, span, quoted_token_len(msg)) {
            eprintln!("{}", block);
        }
    } else {
        eprintln!("{}: runtime error: {}", path, msg);
    }
}

/// Parse a "lexer/parse error at L:C: reason" string and report it.
pub fn report_parse_error(path: &str, source: &str, msg: &str) {
    let (line, col, reason) = parse_error_location_or_fallback(msg);
    report_source_error(path, source, &Span { line, col }, &reason);
}

/// Report a collection of lexer/parse errors.
pub fn report_parse_errors(path: &str, source: &str, errors: &[String]) {
    for msg in errors {
        report_parse_error(path, source, msg);
    }
}

fn parse_error_location_or_fallback(msg: &str) -> (usize, usize, String) {
    if let Some(rest) = msg.strip_prefix("parse error at ") {
        parse_error_location(rest, msg)
    } else if let Some(rest) = msg.strip_prefix("lexer error at ") {
        parse_error_location(rest, msg)
    } else {
        (1, 1, msg.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_points_at_exact_source_column() {
        // "show 0/0." — '/' is at 1-indexed column 7.
        let source = "show 0/0.";
        let span = Span { line: 1, col: 7 };
        let block = source_caret_block(source, &span, 1).expect("caret block should be generated");
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines[0], "    1 | show 0/0.");
        let caret_line = lines[1];
        // The '^' should be directly under the '/' in the source line above.
        let slash_pos = lines[0].find('/').unwrap_or(0);
        let caret_pos = caret_line.find('^').unwrap_or(0);
        assert_eq!(caret_pos, slash_pos);
    }

    #[test]
    fn caret_aligns_for_multi_digit_line_numbers() {
        // Pad source so line 10 contains the expression.
        let source = "show 1.\nshow 2.\nshow 3.\nshow 4.\nshow 5.\nshow 6.\nshow 7.\nshow 8.\nshow 9.\nshow 100 / 0.";
        let span = Span { line: 10, col: 10 };
        let block = source_caret_block(source, &span, 1).expect("caret block should be generated");
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines[0], "    10 | show 100 / 0.");
        let slash_pos = lines[0].find('/').unwrap_or(0);
        let caret_pos = lines[1].find('^').unwrap_or(0);
        assert_eq!(caret_pos, slash_pos);
    }

    #[test]
    fn underline_covers_quoted_token() {
        let source = "show ab.";
        let span = Span { line: 1, col: 6 };
        // "undefined variable 'ab'" should underline both characters.
        let len = quoted_token_len("undefined variable 'ab'");
        assert_eq!(len, 2);
        let block = source_caret_block(source, &span, len).expect("caret block should be generated");
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines[1], "             ^^");
    }
}

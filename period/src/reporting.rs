//! Terminal error-reporting helpers shared by the CLI and REPL.

use crate::ast::Span;

fn source_caret_block(source: &str, span: &Span) -> Option<String> {
    let src_line = source.lines().nth(span.line.saturating_sub(1))?;
    let prefix = format!("    {} | ", span.line);
    let indent = prefix.len() + span.col.saturating_sub(1);
    Some(format!("{}{}\n{}^", prefix, src_line, " ".repeat(indent)))
}

/// Report a source-level error with file location and caret.
pub fn report_source_error(path: &str, source: &str, span: &Span, msg: &str) {
    eprintln!("{}:{}:{}: error: {}", path, span.line, span.col, msg);
    if let Some(block) = source_caret_block(source, span) {
        eprintln!("{}", block);
    }
}

/// Report a source-level warning with file location and caret.
pub fn report_source_warning(path: &str, source: &str, span: &Span, msg: &str) {
    eprintln!("{}:{}:{}: warning: {}", path, span.line, span.col, msg);
    if let Some(block) = source_caret_block(source, span) {
        eprintln!("{}", block);
    }
}

/// Report a runtime error produced by the interpreter.
pub fn report_runtime_error(path: &str, source: &str, msg: &str, span: Option<&Span>) {
    if let Some(span) = span {
        eprintln!("{}:{}:{}: runtime error: {}", path, span.line, span.col, msg);
        if let Some(block) = source_caret_block(source, span) {
            eprintln!("{}", block);
        }
    } else {
        eprintln!("{}: runtime error: {}", path, msg);
    }
}

/// Parse a "lexer/parse error at L:C: reason" string and report it.
pub fn report_parse_error(path: &str, source: &str, msg: &str) {
    let (line, col, reason) = if let Some(rest) = msg.strip_prefix("parse error at ") {
        parse_error_location(rest, msg)
    } else if let Some(rest) = msg.strip_prefix("lexer error at ") {
        parse_error_location(rest, msg)
    } else {
        (1, 1, msg.to_string())
    };

    report_source_error(path, source, &Span { line, col }, &reason);
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
        let block = source_caret_block(source, &span).unwrap();
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines[0], "    1 | show 0/0.");
        let caret_line = lines[1];
        // The '^' should be directly under the '/' in the source line above.
        let slash_pos = lines[0].find('/').unwrap();
        let caret_pos = caret_line.find('^').unwrap();
        assert_eq!(caret_pos, slash_pos);
    }

    #[test]
    fn caret_aligns_for_multi_digit_line_numbers() {
        // Pad source so line 10 contains the expression.
        let source = "show 1.\nshow 2.\nshow 3.\nshow 4.\nshow 5.\nshow 6.\nshow 7.\nshow 8.\nshow 9.\nshow 100 / 0.";
        let span = Span { line: 10, col: 10 };
        let block = source_caret_block(source, &span).unwrap();
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines[0], "    10 | show 100 / 0.");
        let slash_pos = lines[0].find('/').unwrap();
        let caret_pos = lines[1].find('^').unwrap();
        assert_eq!(caret_pos, slash_pos);
    }
}

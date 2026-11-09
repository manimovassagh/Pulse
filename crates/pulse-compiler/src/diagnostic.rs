//! Compiler errors with source locations.

use crate::ast::Span;
use std::fmt;

/// A compile error pointing at a span of the source.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Diagnostic { message: message.into(), span }
    }

    /// Renders the error with file, line, column and a caret under the
    /// offending code:
    ///
    /// ```text
    /// error: type mismatch: expected number, found string
    ///  --> App.tsx:3:30
    ///   |
    /// 3 | const count = signal<number>("zero");
    ///   |                              ^^^^^^
    /// ```
    pub fn render(&self, file: &str, source: &str) -> String {
        let start = self.span.start.min(source.len());
        let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = source[start..].find('\n').map_or(source.len(), |i| start + i);
        let line_no = source[..start].matches('\n').count() + 1;
        let col = source[line_start..start].chars().count() + 1;
        let line = &source[line_start..line_end];
        let width = source[start..self.span.end.clamp(start, line_end)].chars().count().max(1);
        let gutter = " ".repeat(line_no.to_string().len());
        format!(
            "error: {msg}\n{gutter}--> {file}:{line_no}:{col}\n{gutter} |\n{line_no} | {line}\n{gutter} | {pad}{carets}\n",
            msg = self.message,
            pad = " ".repeat(col - 1),
            carets = "^".repeat(width),
        )
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.span.start)
    }
}

impl std::error::Error for Diagnostic {}

pub type Result<T> = std::result::Result<T, Diagnostic>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_caret_under_span() {
        let src = "const a = 1;\nconst b = oops;\n";
        let start = src.find("oops").unwrap();
        let d = Diagnostic::new("unknown name `oops`", Span::new(start, start + 4));
        let out = d.render("App.tsx", src);
        assert!(out.contains("App.tsx:2:11"), "{out}");
        assert!(out.contains("2 | const b = oops;"), "{out}");
        assert!(out.contains("  |           ^^^^"), "{out}");
    }
}

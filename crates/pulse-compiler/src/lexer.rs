//! On-demand tokenizer for TypeScript + JSX.
//!
//! JSX makes tokenization context-sensitive (text between tags is not made of
//! JavaScript tokens), so the lexer does not produce a token stream up front.
//! The parser asks for the next JavaScript token with [`Lexer::next_token`],
//! or for raw JSX pieces with [`Lexer::jsx_text`] / [`Lexer::jsx_name`], and
//! can rewind to any earlier position for speculative parsing.

use crate::ast::Span;
use crate::diagnostic::{Diagnostic, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Punct(&'static str),
    Eof,
}

/// Punctuators, longest first so that e.g. `===` wins over `==`.
const PUNCTS: &[&str] = &[
    "===", "!==", "...", "=>", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=",
    "-=", "*=", "/=", "%=", "(", ")", "{", "}", "[", "]", ";", ",", ".", "<", ">", "+", "-", "*",
    "/", "%", "=", "!", "?", ":", "&", "|",
];

pub struct Lexer<'a> {
    src: &'a str,
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { src, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Skips whitespace and `//` / `/* */` comments.
    pub fn skip_trivia(&mut self) -> Result<()> {
        loop {
            let rest = self.rest();
            let trimmed = rest.trim_start();
            self.pos += rest.len() - trimmed.len();
            if trimmed.starts_with("//") {
                self.pos += trimmed.find('\n').unwrap_or(trimmed.len());
            } else if let Some(body) = trimmed.strip_prefix("/*") {
                match body.find("*/") {
                    Some(end) => self.pos += end + 4,
                    None => {
                        return Err(Diagnostic::new(
                            "unterminated comment",
                            Span::new(self.pos, self.src.len()),
                        ))
                    }
                }
            } else {
                return Ok(());
            }
        }
    }

    /// Reads the next JavaScript/TypeScript token.
    pub fn next_token(&mut self) -> Result<(Tok, Span)> {
        self.skip_trivia()?;
        let start = self.pos;
        let Some(c) = self.peek_char() else {
            return Ok((Tok::Eof, Span::new(start, start)));
        };
        let tok = if is_ident_start(c) {
            let len = self.rest().find(|c: char| !is_ident_part(c)).unwrap_or(self.rest().len());
            let word = &self.rest()[..len];
            self.pos += len;
            Tok::Ident(word.to_string())
        } else if c.is_ascii_digit() || (c == '.' && self.rest()[1..].starts_with(|c: char| c.is_ascii_digit())) {
            self.number()?
        } else if c == '"' || c == '\'' {
            self.string(c)?
        } else if c == '`' {
            return Err(Diagnostic::new(
                "template literals are not supported yet; use `+` to concatenate strings",
                Span::new(start, start + 1),
            ));
        } else if let Some(p) = PUNCTS.iter().find(|p| self.rest().starts_with(**p)) {
            self.pos += p.len();
            Tok::Punct(p)
        } else {
            return Err(Diagnostic::new(
                format!("unexpected character `{c}`"),
                Span::new(start, start + c.len_utf8()),
            ));
        };
        Ok((tok, Span::new(start, self.pos)))
    }

    fn number(&mut self) -> Result<Tok> {
        let start = self.pos;
        let rest = self.rest();
        let (radix, skip) = match rest.get(..2) {
            Some("0x" | "0X") => (16, 2),
            Some("0b" | "0B") => (2, 2),
            Some("0o" | "0O") => (8, 2),
            _ => (10, 0),
        };
        if radix != 10 {
            let digits_len = rest[skip..]
                .find(|c: char| !(c.is_digit(radix) || c == '_'))
                .unwrap_or(rest.len() - skip);
            let digits: String = rest[skip..skip + digits_len].chars().filter(|&c| c != '_').collect();
            self.pos += skip + digits_len;
            return u64::from_str_radix(&digits, radix)
                .map(|v| Tok::Num(v as f64))
                .map_err(|_| Diagnostic::new("invalid number literal", Span::new(start, self.pos)));
        }
        let mut len = 0;
        let bytes = rest.as_bytes();
        let mut seen_dot = false;
        let mut seen_exp = false;
        while len < bytes.len() {
            let b = bytes[len];
            if b.is_ascii_digit() || b == b'_' {
                len += 1;
            } else if b == b'.' && !seen_dot && !seen_exp {
                seen_dot = true;
                len += 1;
            } else if (b == b'e' || b == b'E') && !seen_exp {
                seen_exp = true;
                len += 1;
                if len < bytes.len() && (bytes[len] == b'+' || bytes[len] == b'-') {
                    len += 1;
                }
            } else {
                break;
            }
        }
        let text: String = rest[..len].chars().filter(|&c| c != '_').collect();
        self.pos += len;
        text.parse::<f64>()
            .map(Tok::Num)
            .map_err(|_| Diagnostic::new("invalid number literal", Span::new(start, self.pos)))
    }

    fn string(&mut self, quote: char) -> Result<Tok> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek_char() else {
                return Err(Diagnostic::new("unterminated string", Span::new(start, self.pos)));
            };
            self.pos += c.len_utf8();
            match c {
                c if c == quote => return Ok(Tok::Str(out)),
                '\n' => return Err(Diagnostic::new("unterminated string", Span::new(start, self.pos))),
                '\\' => {
                    let Some(e) = self.peek_char() else { continue };
                    self.pos += e.len_utf8();
                    match e {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        '0' => out.push('\0'),
                        'u' => out.push(self.unicode_escape(start)?),
                        '\n' => {}
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn unicode_escape(&mut self, start: usize) -> Result<char> {
        let err = |end| Diagnostic::new("invalid unicode escape", Span::new(start, end));
        let rest = self.rest();
        let (hex, consumed) = if let Some(inner) = rest.strip_prefix('{') {
            let end = inner.find('}').ok_or_else(|| err(self.pos))?;
            (&inner[..end], end + 2)
        } else {
            (rest.get(..4).ok_or_else(|| err(self.pos))?, 4)
        };
        self.pos += consumed;
        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).ok_or_else(|| err(self.pos))
    }

    /// Reads a JSX tag or attribute name (`div`, `my-element`, `aria-label`).
    pub fn jsx_name(&mut self) -> Result<(String, Span)> {
        self.skip_trivia()?;
        let start = self.pos;
        let len = self
            .rest()
            .find(|c: char| !(is_ident_part(c) || c == '-' || c == ':' || c == '.'))
            .unwrap_or(self.rest().len());
        if len == 0 || !self.rest().starts_with(is_ident_start) {
            return Err(Diagnostic::new("expected a JSX name", Span::new(start, start + 1)));
        }
        self.pos += len;
        Ok((self.src[start..self.pos].to_string(), Span::new(start, self.pos)))
    }

    /// Reads a JSX attribute string value (`"..."`), which has no escapes.
    pub fn jsx_attr_string(&mut self) -> Result<String> {
        self.skip_trivia()?;
        let start = self.pos;
        let quote = self.peek_char().filter(|&c| c == '"' || c == '\'');
        let Some(quote) = quote else {
            return Err(Diagnostic::new("expected a string or `{expression}`", Span::new(start, start + 1)));
        };
        let end = self.rest()[1..]
            .find(quote)
            .ok_or_else(|| Diagnostic::new("unterminated attribute string", Span::new(start, self.src.len())))?;
        let raw = &self.rest()[1..1 + end];
        self.pos += end + 2;
        Ok(decode_entities(raw))
    }

    /// Reads raw text between JSX tags, up to the next `<` or `{`.
    pub fn jsx_text(&mut self) -> (String, Span) {
        let start = self.pos;
        let len = self.rest().find(['<', '{']).unwrap_or(self.rest().len());
        self.pos += len;
        (self.src[start..self.pos].to_string(), Span::new(start, self.pos))
    }

    /// Consumes `s` if the source (after trivia) continues with it.
    pub fn eat(&mut self, s: &str) -> Result<bool> {
        self.skip_trivia()?;
        if self.rest().starts_with(s) {
            self.pos += s.len();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// True if the raw source at the current position starts with `s`.
    pub fn at(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Decodes the HTML entities that commonly appear in JSX text.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => name
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((ch, end + 1))
        });
        match decoded {
            Some((ch, len)) => {
                out.push(ch);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<Tok> {
        let mut lx = Lexer::new(src);
        let mut out = Vec::new();
        loop {
            let (t, _) = lx.next_token().unwrap();
            if t == Tok::Eof {
                return out;
            }
            out.push(t);
        }
    }

    #[test]
    fn basic_tokens() {
        use Tok::*;
        assert_eq!(
            tokens("const x = a.value++ !== 1_000; // hi\n/* c */ 'it\\'s'"),
            vec![
                Ident("const".into()),
                Ident("x".into()),
                Punct("="),
                Ident("a".into()),
                Punct("."),
                Ident("value".into()),
                Punct("++"),
                Punct("!=="),
                Num(1000.0),
                Punct(";"),
                Str("it's".into()),
            ]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(tokens("0x1F 1.5e3 .5 0b11"), vec![Tok::Num(31.0), Tok::Num(1500.0), Tok::Num(0.5), Tok::Num(3.0)]);
    }

    #[test]
    fn string_escapes() {
        assert_eq!(tokens(r#""a\nbA\u{1F600}""#), vec![Tok::Str("a\nbA😀".into())]);
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a &amp; b &lt;3 &#65;&#x42; &bogus"), "a & b <3 AB &bogus");
    }

    #[test]
    fn errors_have_spans() {
        let err = Lexer::new("  \"open").next_token().unwrap_err();
        assert_eq!(err.span.start, 2);
        assert!(Lexer::new("`tpl`").next_token().unwrap_err().message.contains("template"));
    }
}

//! Stage 1: lexing (source text -> tokens).
//!
//! Hand-written scanner. It never stops at the first error: bad input is
//! reported as a diagnostic and scanning continues, so later stages (and the
//! user) still see everything else.

use super::diagnostics::Diagnostic;
use super::span::Span;
use super::token::{Token, TokenKind};
use super::value::Rgba;

pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lx = Lexer { src, chars: src.char_indices().collect(), i: 0, diags: Vec::new() };
    let mut tokens = Vec::new();
    while let Some(t) = lx.next_token() {
        tokens.push(t);
    }
    // The parser relies on a trailing Eof token so it never runs off the end.
    tokens.push(Token { kind: TokenKind::Eof, span: Span::new(src.len(), src.len()) });
    (tokens, lx.diags)
}

struct Lexer<'a> {
    src: &'a str,
    chars: Vec<(usize, char)>,
    i: usize,
    diags: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).map(|c| c.1)
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.i + n).map(|c| c.1)
    }

    /// Byte offset of the current position.
    fn pos(&self) -> usize {
        self.chars.get(self.i).map(|c| c.0).unwrap_or(self.src.len())
    }

    fn error(&mut self, code: &'static str, start: usize, end: usize, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(code, Span::new(start, end), msg));
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => self.i += 1,
                Some('/') if self.peek_at(1) == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.i += 1;
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    let start = self.pos();
                    self.i += 2;
                    let mut closed = false;
                    while let Some(c) = self.peek() {
                        if c == '*' && self.peek_at(1) == Some('/') {
                            self.i += 2;
                            closed = true;
                            break;
                        }
                        self.i += 1;
                    }
                    if !closed {
                        self.error("L003", start, start + 2, "unterminated block comment");
                    }
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        loop {
            self.skip_trivia();
            let start = self.pos();
            let c = self.peek()?;
            let kind = match c {
                '{' => self.single(TokenKind::LBrace),
                '}' => self.single(TokenKind::RBrace),
                '(' => self.single(TokenKind::LParen),
                ')' => self.single(TokenKind::RParen),
                ':' => self.single(TokenKind::Colon),
                ';' => self.single(TokenKind::Semi),
                ',' => self.single(TokenKind::Comma),
                '=' => self.single(TokenKind::Eq),
                '+' => self.single(TokenKind::Plus),
                '-' => self.single(TokenKind::Minus),
                '*' => self.single(TokenKind::Star),
                '/' => self.single(TokenKind::Slash),
                '"' => self.string(start),
                '#' => self.color(start),
                c if c.is_ascii_digit() => self.number(start),
                c if c.is_alphabetic() || c == '_' => self.ident(start),
                other => {
                    self.i += 1;
                    let end = self.pos();
                    self.error("L001", start, end, format!("unexpected character '{other}'"));
                    None
                }
            };
            // `None` means "reported an error and produced no token": keep scanning.
            if let Some(kind) = kind {
                return Some(Token { kind, span: Span::new(start, self.pos()) });
            }
        }
    }

    fn single(&mut self, kind: TokenKind) -> Option<TokenKind> {
        self.i += 1;
        Some(kind)
    }

    fn ident(&mut self, start: usize) -> Option<TokenKind> {
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.i += 1;
        }
        let text = &self.src[start..self.pos()];
        Some(match text {
            "let" => TokenKind::Let,
            "at" => TokenKind::At,
            "video" => TokenKind::Video,
            "group" => TokenKind::Group,
            _ => TokenKind::Ident(text.to_string()),
        })
    }

    fn number(&mut self, start: usize) -> Option<TokenKind> {
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.peek() == Some('.') && matches!(self.peek_at(1), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        let num_end = self.pos();
        let value: f64 = self.src[start..num_end].parse().unwrap_or(0.0);

        // Optional unit suffix glued to the number: `2s`, `500ms`.
        let suffix_start = self.i;
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.i += 1;
        }
        let suffix: String = self.chars[suffix_start..self.i].iter().map(|c| c.1).collect();
        match suffix.as_str() {
            "" => Some(TokenKind::Number(value)),
            "s" => Some(TokenKind::Time(value)),
            "ms" => Some(TokenKind::Time(value / 1000.0)),
            other => {
                let end = self.pos();
                self.diags.push(
                    Diagnostic::error("L004", Span::new(num_end, end), format!("invalid suffix '{other}' on number"))
                        .with_help("time values use 's' or 'ms', e.g. 2s or 500ms"),
                );
                // Still return the number so the parser can carry on.
                Some(TokenKind::Number(value))
            }
        }
    }

    fn color(&mut self, start: usize) -> Option<TokenKind> {
        self.i += 1; // '#'
        let digits_start = self.i;
        while matches!(self.peek(), Some(c) if c.is_alphanumeric()) {
            self.i += 1;
        }
        let digits: String = self.chars[digits_start..self.i].iter().map(|c| c.1).collect();
        let end = self.pos();
        let parsed = if digits.chars().all(|c| c.is_ascii_hexdigit()) {
            match digits.len() {
                // #rgb and #rgba: each digit is doubled (f -> ff)
                3 | 4 => {
                    let d: Vec<u8> = digits.chars().map(|c| c.to_digit(16).unwrap() as u8 * 17).collect();
                    Some(Rgba::new(d[0], d[1], d[2], d.get(3).copied().unwrap_or(255)))
                }
                // #rrggbb and #rrggbbaa
                6 | 8 => {
                    let p = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).unwrap();
                    let a = if digits.len() == 8 { p(6) } else { 255 };
                    Some(Rgba::new(p(0), p(2), p(4), a))
                }
                _ => None,
            }
        } else {
            None
        };
        match parsed {
            Some(c) => Some(TokenKind::Color(c)),
            None => {
                self.diags.push(
                    Diagnostic::error("L005", Span::new(start, end), format!("invalid color '#{digits}'"))
                        .with_help("colors are #rgb, #rgba, #rrggbb or #rrggbbaa, e.g. #f80 or #ff880080"),
                );
                Some(TokenKind::Color(Rgba::rgb(0, 0, 0)))
            }
        }
    }

    fn string(&mut self, start: usize) -> Option<TokenKind> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => {
                    self.error("L002", start, start + 1, "unterminated string");
                    return Some(TokenKind::Str(out));
                }
                Some('"') => {
                    self.i += 1;
                    return Some(TokenKind::Str(out));
                }
                Some('\\') => {
                    let esc_start = self.pos();
                    self.i += 1;
                    match self.peek() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('"') => out.push('"'),
                        Some('\\') => out.push('\\'),
                        Some(c) => {
                            let end = esc_start + 1 + c.len_utf8();
                            self.error("L006", esc_start, end, format!("unknown escape '\\{c}'"));
                        }
                        None => continue,
                    }
                    self.i += 1;
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

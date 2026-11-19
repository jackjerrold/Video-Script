//! Tokens produced by the lexer.

use super::span::Span;
use super::value::Rgba;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    // Literals and names
    Ident(String),
    Number(f64),
    /// A number with a time suffix (`2s`, `500ms`), already converted to seconds.
    Time(f64),
    Color(Rgba),
    Str(String),

    // Keywords. Shape kinds like `rect` are NOT keywords: they are plain
    // identifiers resolved in semantic analysis, so adding a shape never
    // touches the lexer or parser.
    Let,
    AtSign,
    At,
    Video,
    Group,

    // Punctuation
    LBrace,
    RBrace,
    LParen,
    RParen,
    Colon,
    Semi,
    Comma,
    Eq,
    Plus,
    Minus,
    Star,
    Slash,

    Eof,
}

impl TokenKind {
    /// How to name this token in an error message.
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Ident(n) => format!("identifier '{n}'"),
            TokenKind::Number(_) => "a number".into(),
            TokenKind::Time(_) => "a time value".into(),
            TokenKind::Color(_) => "a color".into(),
            TokenKind::Str(_) => "a string".into(),
            TokenKind::Let => "'let'".into(),
            TokenKind::At => "'at'".into(),
            TokenKind::AtSign => "'@'".into(),
            TokenKind::Video => "'video'".into(),
            TokenKind::Group => "'group'".into(),
            TokenKind::LBrace => "'{'".into(),
            TokenKind::RBrace => "'}'".into(),
            TokenKind::LParen => "'('".into(),
            TokenKind::RParen => "')'".into(),
            TokenKind::Colon => "':'".into(),
            TokenKind::Semi => "';'".into(),
            TokenKind::Comma => "','".into(),
            TokenKind::Eq => "'='".into(),
            TokenKind::Plus => "'+'".into(),
            TokenKind::Minus => "'-'".into(),
            TokenKind::Star => "'*'".into(),
            TokenKind::Slash => "'/'".into(),
            TokenKind::Eof => "end of file".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

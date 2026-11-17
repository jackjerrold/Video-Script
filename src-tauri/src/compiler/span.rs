//! Source locations.
//!
//! A [`Span`] is a byte range into the source text. Every token, AST node and
//! diagnostic carries one, so errors can always point at the exact text that
//! caused them. [`Source`] owns the text and converts byte offsets into
//! human-friendly `line:column` positions.

use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// The span covering `self` through the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start, end: other.end }
    }
}

// Compact debug output keeps `{:#?}` dumps of the AST readable.
impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

pub struct Source {
    pub text: String,
    line_starts: Vec<usize>,
}

impl Source {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Source { text, line_starts }
    }

    /// 1-based (line, column) for a byte offset. Columns count characters.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.line_starts[line];
        let col = self
            .text
            .get(start..offset)
            .map(|s| s.chars().count())
            .unwrap_or(offset - start);
        (line + 1, col + 1)
    }

    /// The text of a 1-based line, without its trailing newline.
    pub fn line_text(&self, line: usize) -> &str {
        let line = line.clamp(1, self.line_starts.len());
        let start = self.line_starts[line - 1];
        let end = self.line_starts.get(line).copied().unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(|c| c == '\n' || c == '\r')
    }
}

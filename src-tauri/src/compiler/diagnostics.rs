//! Errors and warnings.
//!
//! Every stage reports problems as [`Diagnostic`]s instead of failing on the
//! first one, so a user (or an LLM) sees as many problems as possible per run.
//!
//! Codes: `L***` lexer, `P***` parser, `S***` semantic analysis,
//! `W***` warnings. Keep codes stable: tools and prompts can key off them.

use super::span::{Source, Span};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Error, code, message: message.into(), span, help: None }
    }

    pub fn warning(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Warning, code, message: message.into(), span, help: None }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// rustc-style multi-line rendering with a source snippet and caret.
    pub fn render(&self, src: &Source) -> String {
        let (line, col) = src.line_col(self.span.start);
        let (end_line, end_col) = src.line_col(self.span.end);
        let text = src.line_text(line);
        let len = if end_line == line {
            end_col.saturating_sub(col).max(1)
        } else {
            text.chars().count().saturating_sub(col - 1).max(1)
        };
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let gutter = " ".repeat(line.to_string().len());
        let mut out = format!(
            "{sev}[{code}]: {msg}\n{gutter}--> {line}:{col}\n{gutter} |\n{line} | {text}\n{gutter} | {pad}{carets}\n",
            code = self.code,
            msg = self.message,
            pad = " ".repeat(col - 1),
            carets = "^".repeat(len),
        );
        if let Some(h) = &self.help {
            out.push_str(&format!("{gutter} = help: {h}\n"));
        }
        out
    }

    /// Flat, serializable form for the frontend.
    pub fn view(&self, src: &Source) -> DiagnosticView {
        let (line, col) = src.line_col(self.span.start);
        let (end_line, end_col) = src.line_col(self.span.end);
        DiagnosticView {
            severity: self.severity,
            code: self.code,
            message: self.message.clone(),
            line,
            col,
            end_line,
            end_col,
            help: self.help.clone(),
            rendered: self.render(src),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticView {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub help: Option<String>,
    pub rendered: String,
}

pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

/// "Did you mean ...?" helper: the closest candidate within a small edit distance.
pub fn suggest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let name_l = name.to_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        let d = edit_distance(&name_l, &c.to_lowercase());
        if best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    let limit = if name.chars().count() <= 3 { 1 } else { 2 };
    best.filter(|(d, _)| *d <= limit).map(|(_, c)| c.to_string())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

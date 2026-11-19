//! Inspector parameters: `@slider(0, 100) let speed = 40;`
//!
//! A `let` with an attribute becomes a [`Param`]: a named constant the app can
//! show as a slider / color picker / text box / dropdown. The UI changes a
//! value in two ways:
//!
//! * **overrides**: the app sends `{ name: value }` next to the script and sema
//!   substitutes it for the literal. The script text is untouched.
//! * **baking**: [`bake`] rewrites the literals in the script text so the
//!   current values become the new defaults.

use super::value::{Rgba, Value};
use serde::Serialize;
use std::collections::BTreeMap;

/// Values chosen in the inspector, by variable name (JSON: number or string).
pub type Overrides = BTreeMap<String, serde_json::Value>;

pub const ATTRIBUTES: &[&str] = &["slider", "color", "text", "choice"];

#[derive(Clone, Debug, Serialize)]
pub struct Param {
    pub name: String,
    /// "slider" | "color" | "text" | "choice"
    pub kind: &'static str,
    /// Current value (the override if there is one). Numbers, or strings
    /// (colors as hex).
    pub value: serde_json::Value,
    /// The value written in the script.
    pub default: serde_json::Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub choices: Vec<String>,
    /// Byte range of the value expression in the source, used by [`bake`].
    pub start: usize,
    pub end: usize,
}

pub fn to_json(v: &Value) -> Option<serde_json::Value> {
    match v {
        Value::Number(n) => Some(serde_json::json!(n)),
        Value::Color(c) => Some(serde_json::json!(c.hex())),
        Value::Str(s) => Some(serde_json::json!(s)),
        _ => None,
    }
}

pub fn parse_hex(s: &str) -> Option<Rgba> {
    let d = s.strip_prefix('#')?;
    if !d.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&d[i..i + 2], 16).ok();
    match d.len() {
        6 => Some(Rgba::new(p(0)?, p(2)?, p(4)?, 255)),
        8 => Some(Rgba::new(p(0)?, p(2)?, p(4)?, p(6)?)),
        _ => None,
    }
}

/// Turn a JSON override into a value of the right kind, or `None` if it does
/// not fit (wrong type, not finite, not one of the choices). Bad overrides
/// are ignored, so a stale one never breaks a script.
pub fn from_override(kind: &str, choices: &[String], ov: &serde_json::Value) -> Option<Value> {
    match kind {
        "slider" => ov.as_f64().filter(|n| n.is_finite()).map(Value::Number),
        "color" => parse_hex(ov.as_str()?).map(Value::Color),
        "text" => Some(Value::Str(ov.as_str()?.to_string())),
        "choice" => {
            let s = ov.as_str()?;
            choices.iter().any(|c| c == s).then(|| Value::Str(s.to_string()))
        }
        _ => None,
    }
}

fn literal(p: &Param) -> Option<String> {
    Some(match (p.kind, &p.value) {
        ("slider", serde_json::Value::Number(n)) => {
            let n = n.as_f64()?;
            let s = format!("{:.6}", n);
            let s = s.trim_end_matches('0').trim_end_matches('.');
            if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
        }
        ("color", serde_json::Value::String(s)) => s.clone(),
        (_, serde_json::Value::String(s)) => {
            let esc = s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
            format!("\"{esc}\"")
        }
        _ => return None,
    })
}

/// Rewrite the script so every parameter whose value differs from its
/// default holds its current value. Everything else is left byte-for-byte.
pub fn bake(src: &str, params: &[Param]) -> String {
    let mut edits: Vec<(usize, usize, String)> = params
        .iter()
        .filter(|p| p.value != p.default)
        .filter_map(|p| literal(p).map(|l| (p.start, p.end, l)))
        .filter(|(s, e, _)| s <= e && *e <= src.len() && src.is_char_boundary(*s) && src.is_char_boundary(*e))
        .collect();
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = src.to_string();
    for (s, e, l) in edits {
        out.replace_range(s..e, &l);
    }
    out
}

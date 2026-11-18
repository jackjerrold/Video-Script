//! The compiler pipeline.
//!
//! ```text
//!  source text
//!     |  lexer      tokens                    (lexer.rs, token.rs)
//!     v
//!  tokens
//!     |  parser     abstract syntax tree      (parser.rs, ast.rs)
//!     v
//!  AST
//!     |  sema       checks + constant folding (sema.rs, schema.rs, builtins.rs)
//!     v
//!  IR               keyframe timelines        (ir.rs)
//!     |  optimize   IR -> smaller IR          (optimize.rs)
//!     v
//!  IR
//!     |  eval       IR @ time t -> Frame      (eval.rs, geom.rs)
//!     v
//!  Frame
//!     |  backend    Frame -> pixels           (backend/raster.rs, backend/text.rs)
//!     v
//!  video (via ffmpeg, in ../video.rs)
//! ```
//!
//! Every stage reports problems as [`Diagnostic`]s instead of panicking, and
//! the pipeline stops before semantic analysis if lexing or parsing failed.

pub mod ast;
pub mod backend;
pub mod builtins;
pub mod diagnostics;
pub mod eval;
pub mod geom;
pub mod ir;
pub mod lexer;
pub mod optimize;
pub mod parser;
pub mod schema;
pub mod sema;
pub mod span;
pub mod token;
pub mod value;

#[cfg(test)]
mod tests;

use diagnostics::{has_errors, Diagnostic};
use span::Source;

pub struct Output {
    /// `None` if there were errors. Warnings do not prevent a program.
    pub program: Option<ir::Program>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn compile(source: &Source) -> Output {
    let (tokens, mut diags) = lexer::lex(&source.text);
    let (ast, parse_diags) = parser::parse(tokens);
    diags.extend(parse_diags);
    if has_errors(&diags) {
        return Output { program: None, diagnostics: diags };
    }

    let (program, sema_diags) = sema::analyze(&ast);
    diags.extend(sema_diags);
    let program = program.map(|mut p| {
        optimize::run(&mut p);
        p
    });
    Output { program, diagnostics: diags }
}

/// Text dump of an intermediate stage, for the app's debug tabs and for
/// developing the compiler. `stage` is "tokens", "ast" or "ir".
pub fn debug_stage(source: &Source, stage: &str) -> Result<String, String> {
    match stage {
        "tokens" => {
            let (tokens, diags) = lexer::lex(&source.text);
            let mut out = String::new();
            for t in &tokens {
                out.push_str(&format!("{:>5}..{:<5} {:?}\n", t.span.start, t.span.end, t.kind));
            }
            for d in &diags {
                out.push('\n');
                out.push_str(&d.render(source));
            }
            Ok(out)
        }
        "ast" => {
            let (tokens, mut diags) = lexer::lex(&source.text);
            let (ast, pd) = parser::parse(tokens);
            diags.extend(pd);
            let mut out = format!("{ast:#?}\n");
            for d in &diags {
                out.push('\n');
                out.push_str(&d.render(source));
            }
            Ok(out)
        }
        "ir" => {
            let o = compile(source);
            let mut out = match &o.program {
                Some(p) => serde_json::to_string_pretty(p).map_err(|e| e.to_string())?,
                None => "(no IR: fix the errors first)".to_string(),
            };
            out.push('\n');
            for d in &o.diagnostics {
                out.push('\n');
                out.push_str(&d.render(source));
            }
            Ok(out)
        }
        other => Err(format!("unknown stage '{other}' (use tokens, ast or ir)")),
    }
}

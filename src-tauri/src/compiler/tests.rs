//! Compiler tests. Run with `cargo test` inside `src-tauri/`.
//! Pattern for new features: write a tiny script, compile it, assert on the
//! diagnostics or on `eval_prop` at a few times.

use super::diagnostics::{has_errors, Severity};
use super::eval::{eval, eval_prop};
use super::ir::Program;
use super::span::Source;
use super::token::TokenKind;
use super::value::{Rgb, Value};
use super::{compile, lexer};

const HEADER: &str = "video { width: 200; height: 100; fps: 10; duration: 4s; }\n";

fn build(body: &str) -> (Option<Program>, Vec<(String, Severity)>) {
    let src = Source::new(format!("{HEADER}{body}"));
    let out = compile(&src);
    let codes = out.diagnostics.iter().map(|d| (d.code.to_string(), d.severity)).collect();
    (out.program, codes)
}

fn ok(body: &str) -> Program {
    let (p, d) = build(body);
    assert!(p.is_some(), "expected success, got {d:?}");
    p.unwrap()
}

fn error_codes(body: &str) -> Vec<String> {
    let (p, d) = build(body);
    assert!(p.is_none(), "expected failure");
    d.into_iter().filter(|(_, s)| *s == Severity::Error).map(|(c, _)| c).collect()
}

fn num(p: &Program, shape: &str, prop: &str, t: f64) -> f64 {
    let s = p.shapes.iter().find(|s| s.name == shape).unwrap();
    let pr = s.props.iter().find(|p| p.name == prop).unwrap();
    match eval_prop(pr, t) {
        Value::Number(n) => n,
        v => panic!("not a number: {v:?}"),
    }
}

// ---- lexer ----------------------------------------------------------------

#[test]
fn lexes_literals() {
    let (toks, diags) = lexer::lex("let a = 2s + 500ms * #f80 \"hi\\n\" // comment\n/* block */ x");
    assert!(diags.is_empty(), "{diags:?}");
    let kinds: Vec<_> = toks.into_iter().map(|t| t.kind).collect();
    assert_eq!(kinds[0], TokenKind::Let);
    assert_eq!(kinds[3], TokenKind::Time(2.0));
    assert_eq!(kinds[5], TokenKind::Time(0.5));
    assert_eq!(kinds[7], TokenKind::Color(Rgb(255, 136, 0)));
    assert_eq!(kinds[8], TokenKind::Str("hi\n".into()));
    assert_eq!(kinds[9], TokenKind::Ident("x".into()));
    assert_eq!(kinds[10], TokenKind::Eof);
}

#[test]
fn lexer_reports_but_continues() {
    let (toks, diags) = lexer::lex("a @ b 3xyz #12 \"oops");
    let codes: Vec<_> = diags.iter().map(|d| d.code).collect();
    assert_eq!(codes, ["L001", "L004", "L005", "L002"]);
    assert!(toks.len() > 4); // kept scanning
}

// ---- parser ---------------------------------------------------------------

#[test]
fn parser_recovers_and_reports_multiple_errors() {
    let src = Source::new("video { width: ; }\nrect { x: 1; }\nlet a = 5\nlet b = 6;");
    let out = compile(&src);
    assert!(out.program.is_none());
    assert!(out.diagnostics.len() >= 3, "{:?}", out.diagnostics);
}

#[test]
fn last_property_may_omit_semicolon() {
    ok("rect a { x: 1; y: 2 }");
}

// ---- semantic analysis -----------------------------------------------------

#[test]
fn constant_folding_and_builtins() {
    let p = ok("let half = width / 2;\nrect a { x: half + 10; y: height * 2 - 1; w: max(3, 7); }");
    assert_eq!(num(&p, "a", "x", 0.0), 110.0);
    assert_eq!(num(&p, "a", "y", 0.0), 199.0);
    assert_eq!(num(&p, "a", "w", 0.0), 7.0);
}

#[test]
fn interpolates_from_declared_value() {
    let p = ok("rect a { x: 10; }\nat 2s a { x: 50; }");
    assert_eq!(num(&p, "a", "x", 0.0), 10.0);
    assert_eq!(num(&p, "a", "x", 1.0), 30.0); // halfway between base and key
    assert_eq!(num(&p, "a", "x", 2.0), 50.0);
    assert_eq!(num(&p, "a", "x", 3.5), 50.0); // holds after the last key
}

#[test]
fn easing_applies_to_the_segment_ending_at_the_key() {
    let p = ok("rect a { x: 0; }\nat 2s a ease in { x: 100; }");
    assert_eq!(num(&p, "a", "x", 1.0), 25.0); // (0.5)^2 * 100
}

#[test]
fn colors_interpolate() {
    let p = ok("rect a { fill: #000000; }\nat 2s a { fill: #646464; }");
    let s = &p.shapes[0];
    let fill = s.props.iter().find(|p| p.name == "fill").unwrap();
    assert_eq!(eval_prop(fill, 1.0), Value::Color(Rgb(50, 50, 50)));
}

#[test]
fn error_codes_are_stable() {
    assert_eq!(error_codes("rekt a { }"), ["S001"]);
    assert_eq!(error_codes("rect a { }\nrect a { }"), ["S002"]);
    assert_eq!(error_codes("at 1s nope { x: 1; }"), ["S003"]);
    assert_eq!(error_codes("rect a { colr: #fff; }"), ["S004"]);
    assert_eq!(error_codes("rect a { x: #fff; }"), ["S005"]);
    assert_eq!(error_codes("rect a { }\nat 1s a { x: 1; }\nat 1s a { x: 2; }"), ["S006"]);
    assert_eq!(error_codes("rect a { x: nothing; }"), ["S007"]);
    assert_eq!(error_codes("rect a { x: 1 / 0; }"), ["S008"]);
    assert_eq!(error_codes("rect a { }\nat 1s a ease wobble { x: 1; }"), ["S016"]);
}

#[test]
fn failed_declarations_do_not_cascade() {
    // Only the bad kind is reported, not every later use of the shape.
    assert_eq!(error_codes("rekt a { }\nat 1s a { x: 1; }\nat 2s a { x: 2; }"), ["S001"]);
}

#[test]
fn missing_and_invalid_video() {
    let src = Source::new("rect a { }");
    assert!(has_errors(&compile(&src).diagnostics));
    let src = Source::new("video { width: 0; }");
    assert!(has_errors(&compile(&src).diagnostics));
    let src = Source::new("video { }\nvideo { }");
    assert!(has_errors(&compile(&src).diagnostics));
}

#[test]
fn warnings_do_not_block_compilation() {
    let (p, d) = build("let unused = 1;\nrect a { opacity: 2; }\nat 9s a { x: 1; }");
    assert!(p.is_some());
    let codes: Vec<_> = d.iter().map(|(c, _)| c.as_str()).collect();
    assert!(codes.contains(&"W001") && codes.contains(&"W002") && codes.contains(&"W003"), "{codes:?}");
}

#[test]
fn suggestions_are_offered() {
    let src = Source::new(format!("{HEADER}rect box {{ }}\nat 1s bax {{ x: 1; }}"));
    let out = compile(&src);
    let help = out.diagnostics[0].help.clone().unwrap();
    assert!(help.contains("box"), "{help}");
}

// ---- optimizer ---------------------------------------------------------------

#[test]
fn optimizer_collapses_constant_tracks() {
    let p = ok("rect a { x: 5; }\nat 1s a { x: 5; }\nat 2s a { x: 5; }");
    let x = p.shapes[0].props.iter().find(|p| p.name == "x").unwrap();
    assert!(x.keys.is_empty());
}

#[test]
fn optimizer_drops_redundant_middle_keys() {
    let p = ok("rect a { x: 0; }\nat 1s a { x: 10; }\nat 2s a { x: 10; }\nat 3s a { x: 10; }\nat 4s a { x: 20; }");
    let x = p.shapes[0].props.iter().find(|p| p.name == "x").unwrap();
    let times: Vec<f64> = x.keys.iter().map(|k| k.time).collect();
    assert_eq!(times, [0.0, 1.0, 3.0, 4.0]);
}

// ---- eval + backend ---------------------------------------------------------------

#[test]
fn renders_pixels() {
    let p = ok("rect a { x: 100; y: 50; w: 40; h: 40; fill: #ff0000; }");
    let pm = super::backend::raster::draw(&eval(&p, 0.0));
    assert_eq!((pm.width(), pm.height()), (200, 100));
    let px = pm.pixel(100, 50).unwrap();
    assert_eq!((px.red(), px.green(), px.blue()), (255, 0, 0));
    let bg = pm.pixel(2, 2).unwrap();
    assert_eq!((bg.red(), bg.green(), bg.blue()), (20, 20, 28));
}

#[test]
fn zero_sizes_do_not_panic() {
    let p = ok("rect a { w: 0; h: 0; }\ncircle c { r: 0; }");
    super::backend::raster::draw(&eval(&p, 0.0));
}

#[test]
fn ir_serializes_to_json() {
    let p = ok("rect a { x: 1; fill: #ff0000; }\nat 2s a { x: 9; }");
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"#ff0000\"") && json.contains("\"kind\":\"rect\""), "{json}");
}

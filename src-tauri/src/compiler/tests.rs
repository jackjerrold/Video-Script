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
    let s = p.find(shape).unwrap();
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

// ---- relative keyframes -------------------------------------------------------

#[test]
fn relative_keyframes_accumulate() {
    let p = ok("rect a { x: 100; }\nat 1s a { x: x + 50; }\nat 2s a { x: x + 50; }");
    assert_eq!(num(&p, "a", "x", 0.0), 100.0);
    assert_eq!(num(&p, "a", "x", 1.0), 150.0);
    assert_eq!(num(&p, "a", "x", 2.0), 200.0);
    assert_eq!(num(&p, "a", "x", 1.5), 175.0);
}

#[test]
fn relative_keyframes_resolve_in_time_order_not_source_order() {
    let p = ok("rect a { x: 100; }\nat 2s a { x: x + 50; }\nat 1s a { x: x + 50; }");
    assert_eq!(num(&p, "a", "x", 1.0), 150.0);
    assert_eq!(num(&p, "a", "x", 2.0), 200.0);
}

#[test]
fn relative_expressions_can_read_other_properties_and_swap() {
    let p = ok("rect a { x: 10; y: 20; }\nat 1s a { x: y; y: x; }");
    assert_eq!(num(&p, "a", "x", 1.0), 20.0);
    assert_eq!(num(&p, "a", "y", 1.0), 10.0);
}

#[test]
fn relative_expressions_mix_with_math_and_builtins() {
    // width is 200 in the test header
    let p = ok("rect a { x: 10; }\nat 1s a { x: max(x * 3, 5) + width / 10; }");
    assert_eq!(num(&p, "a", "x", 1.0), 50.0);
}

#[test]
fn property_names_beat_variables_inside_at_blocks() {
    let (p, d) = build("let x = 999;\nrect a { x: 1; }\nat 1s a { x: x + 1; }");
    let p = p.expect("compiles");
    assert_eq!(num(&p, "a", "x", 1.0), 2.0); // the property, not the variable
    assert!(d.iter().any(|(c, _)| c == "W004"), "{d:?}");
}

#[test]
fn relative_keyframes_respect_per_property_ease() {
    let p = ok("rect a { x: 0; }\nat 2s a ease in { x: x + 100; }");
    assert_eq!(num(&p, "a", "x", 1.0), 25.0);
}

#[test]
fn relative_errors() {
    // property names mean "previous value", so they only exist inside `at`
    assert_eq!(error_codes("rect a { x: 5; y: x; }"), ["S007"]);
    // a division that is only invalid once the previous value is known
    assert_eq!(error_codes("rect a { x: 5; }\nat 1s a { x: 10 / (x - x); }"), ["S008"]);
    // type checks work on symbolic values too
    assert_eq!(error_codes("rect a { }\nat 1s a { x: fill + 1; }"), ["S005"]);
    assert_eq!(error_codes("rect a { }\nat 1s a { fill: x; }"), ["S005"]);
}

// ---- groups ---------------------------------------------------------------------------

fn frame_world(p: &Program, t: f64, i: usize) -> (f64, f64) {
    eval(p, t).shapes[i].world.apply(0.0, 0.0)
}

#[test]
fn group_transform_applies_to_children() {
    let p = ok("group g { x: 100; y: 50; rect r { x: 10; w: 20; h: 20; fill: #ff0000; } }");
    let f = eval(&p, 0.0);
    assert_eq!(f.shapes.len(), 1); // the group itself is not drawn
    assert_eq!(frame_world(&p, 0.0, 0), (110.0, 50.0));
    let pm = super::backend::raster::draw(&f);
    let px = pm.pixel(110, 50).unwrap();
    assert_eq!((px.red(), px.green(), px.blue()), (255, 0, 0));
}

#[test]
fn group_rotation_and_scale_apply_about_the_group_origin() {
    // 90 degrees clockwise: a child at local (30, 0) ends up 30px below the origin
    let p = ok("group g { x: 100; y: 20; rot: 90; rect r { x: 30; } }");
    let (x, y) = frame_world(&p, 0.0, 0);
    assert!((x - 100.0).abs() < 1e-9 && (y - 50.0).abs() < 1e-9, "{x},{y}");

    let p = ok("group g { x: 10; scale: 2; rect r { x: 10; } }");
    assert_eq!(frame_world(&p, 0.0, 0), (30.0, 0.0));
}

#[test]
fn animating_a_group_moves_all_children_together() {
    let p = ok("group g { rect a { x: 0; } rect b { x: 50; } }\nat 2s g { x: 100; }");
    assert_eq!(frame_world(&p, 1.0, 0).0, 50.0);
    assert_eq!(frame_world(&p, 1.0, 1).0, 100.0);
    assert_eq!(frame_world(&p, 2.0, 0).0, 100.0);
    assert_eq!(frame_world(&p, 2.0, 1).0, 150.0);
}

#[test]
fn nested_groups_compose_and_opacity_multiplies() {
    let p = ok("group outer { x: 10; opacity: 0.5; group inner { x: 20; opacity: 0.5; rect r { x: 5; } } }");
    let f = eval(&p, 0.0);
    assert_eq!(f.shapes[0].world.apply(0.0, 0.0).0, 35.0);
    assert_eq!(f.shapes[0].opacity, 0.25);
}

#[test]
fn children_and_groups_animate_independently_and_relatively() {
    let p = ok("group g { x: 100; rect r { x: 10; } }\nat 1s r { x: x + 5; }\nat 2s g { x: x + 50; }");
    // at t=1 the group is halfway through its own move (100 -> 150) and r is at 10 + 5
    assert_eq!(frame_world(&p, 1.0, 0).0, 125.0 + 15.0);
    assert_eq!(frame_world(&p, 2.0, 0).0, 150.0 + 15.0);
}

#[test]
fn draw_order_is_depth_first_in_declaration_order() {
    let p = ok("rect a { }\ngroup g { rect b { } group h { rect c { } } rect d { } }\nrect e { }");
    let names: Vec<_> = p
        .shapes
        .iter()
        .flat_map(|s| {
            fn walk(s: &super::ir::Shape, out: &mut Vec<String>) {
                out.push(s.name.clone());
                s.children.iter().for_each(|c| walk(c, out));
            }
            let mut v = vec![];
            walk(s, &mut v);
            v
        })
        .collect();
    assert_eq!(names, ["a", "g", "b", "h", "c", "d", "e"]);
    assert_eq!(eval(&p, 0.0).shapes.len(), 5); // groups draw nothing
}

#[test]
fn group_errors() {
    assert_eq!(error_codes("group g { fill: #fff; }"), ["S004"]); // groups have no fill
    assert_eq!(error_codes("group g { }\ngroup g { }"), ["S002"]);
    assert_eq!(error_codes("group g { rect a { } rect a { } }"), ["S002"]); // names are global
    // a rejected group does not make its children's keyframes cascade
    assert_eq!(error_codes("group g { }\ngroup g { rect kid { } }\nat 1s kid { x: 1; }"), ["S002"]);
}

#[test]
fn group_syntax_errors_recover() {
    let src = Source::new(format!("{HEADER}group g {{ x: 1; at 1s g {{ x: 2; }} rect r {{ }} }}\nrect z {{ y: ; }}"));
    let out = compile(&src);
    assert!(out.program.is_none());
    assert!(out.diagnostics.len() >= 2, "{:?}", out.diagnostics);
}

#[test]
fn degenerate_group_scale_does_not_panic() {
    let p = ok("group g { scale: 0; rect r { } text t { text: \"x\"; } }");
    super::backend::raster::draw(&eval(&p, 0.0));
}

#[test]
fn json_contains_children() {
    let p = ok("group g { rect r { } }");
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"children\"") && json.contains("\"kind\":\"group\""), "{json}");
}

// ---- text -------------------------------------------------------------------------------

/// Bounding box (min_x, max_x, min_y, max_y) of pixels that differ clearly from the background.
fn lit_bounds(pm: &tiny_skia::Pixmap) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
    let mut any = false;
    for y in 0..pm.height() {
        for x in 0..pm.width() {
            let p = pm.pixel(x, y).unwrap();
            if p.green() > 120 {
                any = true;
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    any.then_some((x0, x1, y0, y1))
}

#[test]
fn text_renders_and_is_centered_on_its_position() {
    let p = ok("text t { text: \"HELLO\"; size: 30; x: 100; y: 50; fill: #00ff00; }");
    let pm = super::backend::raster::draw(&eval(&p, 0.0));
    let (x0, x1, y0, y1) = lit_bounds(&pm).expect("text drew something");
    let cx = (x0 + x1) as f64 / 2.0;
    let cy = (y0 + y1) as f64 / 2.0;
    assert!((cx - 100.0).abs() < 8.0, "horizontal center {cx}");
    assert!((cy - 50.0).abs() < 12.0, "vertical center {cy}");
    assert!(x1 - x0 > 50, "text too narrow: {}..{}", x0, x1);
}

#[test]
fn text_scales_with_size_and_multiline_is_taller() {
    let height = |body: &str| {
        let p = ok(body);
        let pm = super::backend::raster::draw(&eval(&p, 0.0));
        let (_, _, y0, y1) = lit_bounds(&pm).unwrap();
        y1 - y0
    };
    let small = height("text t { text: \"A\"; size: 20; x: 100; y: 50; fill: #00ff00; }");
    let big = height("text t { text: \"A\"; size: 40; x: 100; y: 50; fill: #00ff00; }");
    let two = height("text t { text: \"A\\nA\"; size: 20; x: 100; y: 50; fill: #00ff00; }");
    assert!(big > small * 3 / 2, "{small} vs {big}");
    assert!(two > small * 3 / 2, "{small} vs {two}");
}

#[test]
fn text_can_change_over_time_and_move_relatively() {
    let p = ok("text t { text: \"Hi\"; x: 10; }\nat 1s t { text: \"Bye\"; x: x + 40; }");
    let s = p.find("t").unwrap();
    let text = s.props.iter().find(|p| p.name == "text").unwrap();
    assert_eq!(eval_prop(text, 0.5), Value::Str("Hi".into()));
    assert_eq!(eval_prop(text, 1.0), Value::Str("Bye".into()));
    assert_eq!(num(&p, "t", "x", 1.0), 50.0);
}

#[test]
fn text_follows_group_transform_and_opacity() {
    let p = ok("group g { x: 100; y: 50; opacity: 0.5; text t { text: \"I\"; size: 40; fill: #ffffff; } }");
    let pm = super::backend::raster::draw(&eval(&p, 0.0));
    let mut max_r = 0;
    for y in 0..pm.height() {
        for x in 0..pm.width() {
            max_r = max_r.max(pm.pixel(x, y).unwrap().red());
        }
    }
    // background 20 blended halfway towards white is about 137
    assert!((120..=150).contains(&max_r), "{max_r}");
}

#[test]
fn text_errors_and_edge_cases() {
    assert_eq!(error_codes("text t { size: \"big\"; }"), ["S005"]);
    assert_eq!(error_codes("text t { text: 5; }"), ["S005"]);
    assert_eq!(error_codes("text t { text: \"a\"; }\nat 1s t { text: text + \"b\"; }"), ["S005"]);
    // empty, whitespace-only and zero-size text draw nothing and do not panic
    let p = ok("text a { text: \"\"; }\ntext b { text: \"   \"; }\ntext c { text: \"x\"; size: 0; }");
    super::backend::raster::draw(&eval(&p, 0.0));
}

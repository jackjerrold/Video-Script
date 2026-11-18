//! Compiler tests. Run with `cargo test` inside `src-tauri/`.
//! Pattern for new features: write a tiny script, compile it, assert on the
//! diagnostics or on `eval_prop` at a few times.

use super::diagnostics::{has_errors, Severity};
use super::eval::{eval, eval_prop};
use super::ir::Program;
use super::span::Source;
use super::token::TokenKind;
use super::value::{Rgba, Value};
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
    assert_eq!(kinds[7], TokenKind::Color(Rgba::rgb(255, 136, 0)));
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
    assert_eq!(eval_prop(fill, 1.0), Value::Color(Rgba::rgb(50, 50, 50)));
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

// ============================================================================
// Colors with alpha, stroke, gradients, text alignment and wrapping
// ============================================================================

use super::backend::text::{layout_lines, text_path, text_width, Align, TextStyle};
use super::value::{Gradient, GradientKind};

const BG: (u8, u8, u8) = (20, 20, 28);

fn render(body: &str) -> tiny_skia::Pixmap {
    super::backend::raster::draw(&eval(&ok(body), 0.0))
}

fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let p = pm.pixel(x, y).unwrap();
    (p.red(), p.green(), p.blue())
}

fn prop_value(p: &Program, shape: &str, prop: &str, t: f64) -> Value {
    let pr = p.find(shape).unwrap().props.iter().find(|p| p.name == prop).unwrap();
    eval_prop(pr, t)
}

// ---- colors with alpha -------------------------------------------------------

#[test]
fn hex_colors_accept_alpha() {
    let (toks, diags) = lexer::lex("#f008 #11223344 #f00 #ff8800");
    assert!(diags.is_empty(), "{diags:?}");
    let kinds: Vec<_> = toks.into_iter().map(|t| t.kind).collect();
    assert_eq!(kinds[0], TokenKind::Color(Rgba::new(255, 0, 0, 136)));
    assert_eq!(kinds[1], TokenKind::Color(Rgba::new(0x11, 0x22, 0x33, 0x44)));
    assert_eq!(kinds[2], TokenKind::Color(Rgba::rgb(255, 0, 0)));
    assert_eq!(kinds[3], TokenKind::Color(Rgba::rgb(255, 136, 0)));
}

#[test]
fn translucent_colors_blend_in_premultiplied_space() {
    // fading red to nothing stays red and just gets more transparent
    let c = Rgba::rgb(255, 0, 0).lerp(Rgba::NONE, 0.5);
    assert_eq!((c.r, c.g, c.b, c.a), (255, 0, 0, 128));
    // fully opaque blends are unchanged
    assert_eq!(Rgba::rgb(0, 0, 0).lerp(Rgba::rgb(100, 100, 100), 0.5), Rgba::rgb(50, 50, 50));
}

#[test]
fn rgba_builtin_and_none() {
    let p = ok("rect a { fill: rgba(255, 0, 0, 0.5); stroke: none; }");
    assert_eq!(prop_value(&p, "a", "fill", 0.0), Value::Color(Rgba::new(255, 0, 0, 128)));
    assert_eq!(prop_value(&p, "a", "stroke", 0.0), Value::Color(Rgba::NONE));
}

#[test]
fn fill_none_draws_nothing_and_translucent_fill_blends() {
    let pm = render("rect a { x: 100; y: 50; w: 40; h: 40; fill: none; }");
    assert_eq!(px(&pm, 100, 50), BG);
    let pm = render("rect a { x: 100; y: 50; w: 40; h: 40; fill: #ffffff80; }");
    let (r, _, _) = px(&pm, 100, 50);
    assert!((130..=145).contains(&r), "half-transparent white over dark bg: {r}");
}

// ---- stroke -------------------------------------------------------------------

#[test]
fn stroke_is_an_outline_centered_on_the_edge() {
    // 40x40 rect at (100,50): its edge is at x=80; a 4px stroke covers x in 78..82
    let pm = render("rect a { x: 100; y: 50; w: 40; h: 40; fill: none; stroke: #ff0000; stroke_width: 4; }");
    assert_eq!(px(&pm, 100, 50), BG, "interior stays empty");
    assert_eq!(px(&pm, 80, 50), (255, 0, 0), "on the edge");
    assert_eq!(px(&pm, 77, 50), BG, "outside the band");
    assert_eq!(px(&pm, 84, 50), BG, "inside, past the band");
}

#[test]
fn fill_and_stroke_draw_together() {
    let pm = render("rect a { x: 100; y: 50; w: 40; h: 40; fill: #00ff00; stroke: #ff0000; stroke_width: 4; }");
    assert_eq!(px(&pm, 100, 50), (0, 255, 0));
    assert_eq!(px(&pm, 80, 50), (255, 0, 0));
}

#[test]
fn no_stroke_without_a_width() {
    let pm = render("rect a { x: 100; y: 50; w: 40; h: 40; fill: none; stroke: #ff0000; }");
    assert_eq!(px(&pm, 80, 50), BG);
}

#[test]
fn stroke_scales_with_its_group() {
    // scale 2 doubles the rect (edge at x=80) and the stroke (4px wide)
    let pm = render(
        "group g { x: 100; y: 50; scale: 2; rect a { w: 20; h: 20; fill: none; stroke: #ff0000; stroke_width: 2; } }",
    );
    assert_eq!(px(&pm, 80, 50), (255, 0, 0));
    assert_eq!(px(&pm, 77, 50), BG);
}

#[test]
fn circles_and_text_can_be_stroked() {
    let pm = render("circle c { x: 100; y: 50; r: 30; fill: none; stroke: #00ff00; stroke_width: 3; }");
    assert_eq!(px(&pm, 100, 50), BG);
    assert_eq!(px(&pm, 130, 50), (0, 255, 0));
    let pm = render("text t { text: \"O\"; size: 60; x: 100; y: 50; fill: none; stroke: #00ff00; stroke_width: 2; }");
    assert!(lit_bounds(&pm).is_some(), "outlined text should draw");
}

#[test]
fn stroke_properties_animate_and_work_relatively() {
    let p = ok("rect a { stroke_width: 2; stroke: #ff0000; }\nat 2s a { stroke_width: stroke_width + 8; stroke: #0000ff; }");
    assert_eq!(num(&p, "a", "stroke_width", 1.0), 6.0);
    assert_eq!(num(&p, "a", "stroke_width", 2.0), 10.0);
    assert_eq!(prop_value(&p, "a", "stroke", 1.0), Value::Color(Rgba::rgb(128, 0, 128)));
}

// ---- gradients ----------------------------------------------------------------------

#[test]
fn linear_gradient_runs_left_to_right() {
    let pm = render("rect a { x: 100; y: 50; w: 100; h: 20; fill: linear(0, #ff0000, #0000ff); }");
    let (r, _, b) = px(&pm, 52, 50);
    assert!(r > 230 && b < 30, "left is red: {r},{b}");
    let (r, _, b) = px(&pm, 147, 50);
    assert!(b > 230 && r < 30, "right is blue: {r},{b}");
    let (r, _, b) = px(&pm, 100, 50);
    assert!((100..155).contains(&r) && (100..155).contains(&b), "middle is mixed: {r},{b}");
}

#[test]
fn linear_gradient_angle_90_runs_top_to_bottom() {
    let pm = render("rect a { x: 100; y: 50; w: 20; h: 80; fill: linear(90, #ff0000, #0000ff); }");
    let (r, _, b) = px(&pm, 100, 12);
    assert!(r > 230 && b < 30, "top is red: {r},{b}");
    let (r, _, b) = px(&pm, 100, 87);
    assert!(b > 230 && r < 30, "bottom is blue: {r},{b}");
}

#[test]
fn gradients_can_have_many_stops() {
    let pm = render("rect a { x: 100; y: 50; w: 100; h: 20; fill: linear(0, #ff0000, #00ff00, #0000ff); }");
    let (r, g, b) = px(&pm, 100, 50);
    assert!(g > 230 && r < 30 && b < 30, "middle stop is green: {r},{g},{b}");
}

#[test]
fn radial_gradient_goes_from_center_outwards() {
    let pm = render("circle c { x: 100; y: 50; r: 40; fill: radial(#ffffff, #000000); }");
    assert!(px(&pm, 100, 50).0 > 240, "center is white");
    assert!(px(&pm, 136, 50).0 < 60, "edge is nearly black");
}

#[test]
fn gradients_follow_the_shapes_rotation() {
    // rotated 90 degrees clockwise, the gradient's left end (red) ends up at the top
    let pm = render("rect a { x: 100; y: 50; w: 80; h: 20; rot: 90; fill: linear(0, #ff0000, #0000ff); }");
    let (r, _, b) = px(&pm, 100, 14);
    assert!(r > 230 && b < 30, "top is red: {r},{b}");
    let (r, _, b) = px(&pm, 100, 86);
    assert!(b > 230 && r < 30, "bottom is blue: {r},{b}");
}

#[test]
fn gradients_can_fade_to_transparent_and_strokes_can_use_them() {
    let pm = render("rect a { x: 100; y: 50; w: 100; h: 20; fill: linear(0, #ffffff, #ffffff00); }");
    assert!(px(&pm, 52, 50).0 > 230);
    assert!(px(&pm, 147, 50).0 < 45, "fades into the background");
    let pm = render(
        "rect a { x: 100; y: 50; w: 100; h: 40; fill: none; stroke: linear(0, #ff0000, #0000ff); stroke_width: 6; }",
    );
    let (r, _, b) = px(&pm, 50, 50); // left edge, on the stroke
    assert!(r > 200 && b < 60, "{r},{b}");
}

#[test]
fn gradients_animate() {
    let p = ok("rect a { fill: linear(0, #000000, #ffffff); }\nat 1s a { fill: linear(90, #000000, #646464); }");
    match prop_value(&p, "a", "fill", 0.5) {
        Value::Gradient(Gradient { kind: GradientKind::Linear { angle }, stops }) => {
            assert_eq!(angle, 45.0);
            assert!((177..=178).contains(&stops[1].r), "{:?}", stops);
        }
        other => panic!("expected a gradient, got {other:?}"),
    }
}

#[test]
fn a_color_blends_into_a_gradient() {
    let p = ok("rect a { fill: #ff0000; }\nat 1s a { fill: linear(0, #00ff00, #0000ff); }");
    match prop_value(&p, "a", "fill", 0.5) {
        Value::Gradient(g) => assert_eq!(g.stops.len(), 2),
        other => panic!("expected a gradient, got {other:?}"),
    }
}

#[test]
fn incompatible_gradients_switch_at_the_keyframe() {
    let p = ok("rect a { fill: linear(0, #f00, #00f); }\nat 1s a { fill: radial(#f00, #0f0, #00f); }");
    assert!(matches!(prop_value(&p, "a", "fill", 0.5), Value::Gradient(Gradient { kind: GradientKind::Linear { .. }, .. })));
    assert!(matches!(prop_value(&p, "a", "fill", 1.0), Value::Gradient(Gradient { kind: GradientKind::Radial, .. })));
}

#[test]
fn gradients_serialize_to_json() {
    let p = ok("rect a { fill: linear(45, #ff0000, #0000ff); }");
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"type\":\"linear\"") && json.contains("\"angle\":45") && json.contains("\"stops\""), "{json}");
}

#[test]
fn paint_and_gradient_errors() {
    assert_eq!(error_codes("rect a { fill: 5; }"), ["S005"]);
    assert_eq!(error_codes("text t { text: \"x\"; fill: \"red\"; }"), ["S005"]);
    assert_eq!(error_codes("rect a { fill: linear(0, #fff); }"), ["S019"]); // needs two colors
    assert_eq!(error_codes("rect a { fill: radial(#fff); }"), ["S019"]);
    assert_eq!(error_codes("rect a { fill: linear(#f00, #00f, #fff); }"), ["S005"]); // angle first
    assert_eq!(error_codes("rect a { fill: radial(5, 6); }"), ["S005"]);
    assert_eq!(error_codes("rect a { w: linear(0, #f00, #00f); }"), ["S005"]);
    assert_eq!(error_codes("rect a { }\nat 1s a { fill: x; }"), ["S005"]);
    assert_eq!(error_codes("group g { stroke: #fff; }"), ["S004"]); // groups are not drawn
    // a negative stroke width is odd but only a warning
    let (p, d) = build("rect a { stroke_width: -1; }");
    assert!(p.is_some() && d.iter().any(|(c, _)| c == "W003"), "{d:?}");
}

// ---- text: alignment, wrapping, line height ------------------------------------------

#[test]
fn align_must_be_a_known_value() {
    assert_eq!(error_codes("text t { align: \"justify\"; }"), ["S020"]);
    assert_eq!(error_codes("text t { }\nat 1s t { align: \"middle\"; }"), ["S020"]);
    let src = Source::new(format!("{HEADER}text t {{ align: \"rigth\"; }}"));
    let out = compile(&src);
    assert!(out.diagnostics[0].help.clone().unwrap().contains("right"));
    ok("text t { align: \"left\"; }\nat 1s t { align: \"right\"; }");
}

#[test]
fn wrapping_respects_the_width_and_keeps_every_word() {
    let s = "the quick brown fox jumps over the lazy dog";
    let lines = layout_lines(s, 20.0, 120.0);
    assert!(lines.len() > 1, "{lines:?}");
    for l in &lines {
        assert!(text_width(l, 20.0) <= 120.5, "'{l}' is too wide");
    }
    assert_eq!(lines.join(" "), s);
}

#[test]
fn wrapping_breaks_words_longer_than_a_line() {
    let w = "Supercalifragilisticexpialidocious";
    let lines = layout_lines(w, 20.0, 100.0);
    assert!(lines.len() > 1);
    for l in &lines {
        assert!(text_width(l, 20.0) <= 100.5, "'{l}' is too wide");
    }
    assert_eq!(lines.concat(), w);
}

#[test]
fn explicit_newlines_and_unwrapped_text_are_preserved() {
    assert_eq!(layout_lines("a\nb", 20.0, 0.0), ["a", "b"]);
    assert_eq!(layout_lines("one  two", 20.0, 0.0), ["one  two"]); // untouched without wrapping
    assert_eq!(layout_lines("aaa\n\nbbb", 20.0, 500.0), ["aaa", "", "bbb"]);
    assert_eq!(layout_lines("hello world", 20.0, 10_000.0), ["hello world"]);
}

fn style(text: &str, align: Align, max_width: f32) -> TextStyle<'_> {
    TextStyle { text, size: 40.0, align, max_width, line_height: 1.0 }
}

#[test]
fn alignment_picks_which_part_of_the_block_sits_on_the_position() {
    let w = text_width("Hello", 40.0);
    let tol = 6.0; // glyph side bearings
    let b = text_path(&style("Hello", Align::Left, 0.0)).unwrap().bounds();
    assert!(b.left().abs() < tol && (b.right() - w).abs() < tol, "left: {}..{}", b.left(), b.right());
    let b = text_path(&style("Hello", Align::Center, 0.0)).unwrap().bounds();
    assert!(((b.left() + b.right()) / 2.0).abs() < tol, "center: {}..{}", b.left(), b.right());
    let b = text_path(&style("Hello", Align::Right, 0.0)).unwrap().bounds();
    assert!(b.right().abs() < tol && (b.left() + w).abs() < tol, "right: {}..{}", b.left(), b.right());
}

#[test]
fn a_wrap_width_defines_the_box_the_alignment_refers_to() {
    let text = "alpha beta gamma delta epsilon zeta";
    let tol = 6.0;
    let b = text_path(&style(text, Align::Left, 100.0)).unwrap().bounds();
    assert!(b.left() > -tol && b.right() <= 100.0 + tol, "left: {}..{}", b.left(), b.right());
    let b = text_path(&style(text, Align::Right, 100.0)).unwrap().bounds();
    assert!(b.left() >= -100.0 - tol && b.right() < tol, "right: {}..{}", b.left(), b.right());
    let b = text_path(&style(text, Align::Center, 100.0)).unwrap().bounds();
    assert!(b.left() >= -50.0 - tol && b.right() <= 50.0 + tol, "center: {}..{}", b.left(), b.right());
}

/// Rightmost and leftmost lit pixel in the rows y0..y1.
fn lit_x_range(pm: &tiny_skia::Pixmap, y0: u32, y1: u32) -> (u32, u32) {
    let (mut lo, mut hi) = (u32::MAX, 0);
    for y in y0..y1 {
        for x in 0..pm.width() {
            if pm.pixel(x, y).unwrap().green() > 120 {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
    }
    (lo, hi)
}

#[test]
fn every_line_is_aligned_to_the_same_edge() {
    // two lines of different widths: right-aligned they share a right edge,
    // left-aligned a left edge
    let right = render("text t { text: \"Hello\\nHi\"; size: 24; x: 180; y: 50; align: \"right\"; fill: #00ff00; }");
    let (_, top) = lit_x_range(&right, 0, 50);
    let (_, bottom) = lit_x_range(&right, 50, 100);
    assert!((top as i32 - bottom as i32).abs() <= 5, "right edges {top} vs {bottom}");
    let left = render("text t { text: \"Hello\\nHi\"; size: 24; x: 20; y: 50; align: \"left\"; fill: #00ff00; }");
    let (top, _) = lit_x_range(&left, 0, 50);
    let (bottom, _) = lit_x_range(&left, 50, 100);
    assert!((top as i32 - bottom as i32).abs() <= 5, "left edges {top} vs {bottom}");
    assert!(top >= 16, "left aligned text starts at its position: {top}");
}

#[test]
fn line_height_scales_the_spacing_between_lines() {
    let h = |lh: f32| {
        let st = TextStyle { text: "A\nA", size: 20.0, align: Align::Center, max_width: 0.0, line_height: lh };
        let b = text_path(&st).unwrap().bounds();
        b.bottom() - b.top()
    };
    assert!(h(2.0) > h(1.0) + 10.0, "{} vs {}", h(1.0), h(2.0));
    assert!(h(0.5) < h(1.0) - 5.0, "{} vs {}", h(0.5), h(1.0));
}

#[test]
fn wrapping_works_end_to_end_and_can_be_animated() {
    let tall = |body: &str| {
        let pm = render(body);
        let (_, _, y0, y1) = lit_bounds(&pm).unwrap();
        y1 - y0
    };
    let one = tall("text t { text: \"aaa bbb ccc ddd\"; size: 14; x: 100; y: 50; fill: #00ff00; }");
    let wrapped = tall("text t { text: \"aaa bbb ccc ddd\"; size: 14; x: 100; y: 50; max_width: 50; fill: #00ff00; }");
    assert!(wrapped > one * 3 / 2, "{one} vs {wrapped}");

    let p = ok("text t { max_width: 100; line_height: 1; }\nat 1s t { max_width: max_width + 40; line_height: line_height * 2; }");
    assert_eq!(num(&p, "t", "max_width", 1.0), 140.0);
    assert_eq!(num(&p, "t", "line_height", 1.0), 2.0);
}

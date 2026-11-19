//! A plain-text language reference, generated from the same tables the
//! compiler uses (`schema.rs`, `builtins.rs`), so it can never go stale.
//! Shown in the app's Reference tab, and handy as context for an LLM that
//! writes scripts.

use super::builtins::BUILTINS;
use super::schema::{self, PropDef, PropDefault, SHAPES, VIDEO_PROPS};
use super::value::Ty;

fn num(v: f64) -> String {
    if v.is_infinite() {
        "∞".into()
    } else if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn describe(p: &PropDef) -> String {
    let ty = match p.ty {
        Ty::Paint => "color | gradient".to_string(),
        t => t.to_string(),
    };
    let default = match p.default {
        PropDefault::Num(n) => num(n),
        PropDefault::Color(c) => if c.a == 0 { "none".into() } else { c.hex() },
        PropDefault::Str(s) => format!("\"{s}\""),
    };
    let mut extra = String::new();
    if let Some((lo, hi)) = p.range {
        if hi.is_infinite() {
            extra = format!("  min {}", num(lo));
        } else if lo != 0.0 || hi != f64::INFINITY {
            extra = format!("  range {} to {}", num(lo), num(hi));
        }
    }
    if let Some(choices) = p.choices {
        extra = format!("  one of {}", choices.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", "));
    }
    format!("  {:<13}{:<18}default {}{}", p.name, ty, default, extra)
}

fn shape_blurb(name: &str) -> &'static str {
    match name {
        "rect" => "rectangle, optionally with rounded corners (radius)",
        "circle" => "circle of radius r",
        "ellipse" => "ellipse with radii rx and ry",
        "line" => "segment from its own origin to (x2, y2), relative to it",
        "polygon" => "regular polygon with `sides` corners, first corner pointing up",
        "star" => "star with `points` tips; `inner` is the inner radius as a fraction of r",
        "text" => "text, positioned by its alignment anchor",
        "group" => "container: its transform and opacity apply to everything inside",
        _ => "",
    }
}

pub fn text() -> String {
    let mut o = String::new();
    o.push_str("KEYFRAME STUDIO LANGUAGE REFERENCE\n\n");
    o.push_str(
        "SYNTAX\n  video { width: 640; height: 360; fps: 30; duration: 5s; background: #101020; }\n  let name = expression;               constants, defined before use\n  <shape> name { property: value; }    declare a shape\n  group name { property: value; <shapes and groups> }\n  at 2s name [ease curve] { property: value [ease curve]; }   keyframe\n  Inside `at`, a property name means its previous value: x: x + 100;\n  Comments: // line   /* block */      Times: 2s, 500ms, or plain seconds\n  Colors: #rgb #rgba #rrggbb #rrggbbaa   Strings: \"text\" with \\n\n\n",
    );

    o.push_str(
        "INSPECTOR PARAMETERS  (an attribute before a `let` shows it as a control in the Inspector tab)\n  @slider(min, max [, step]) let speed = 40;       number: slider\n  @color let accent = #ff5555;                     color picker\n  @text let title = \"Hello\";                       text box\n  @choice(\"sans\", \"mono\") let face = \"sans\";     dropdown\n  Use them like any variable: x: speed; fill: accent; text: title;\n  Changing a control does not touch the script until you press Bake.\n\n",
    );

    o.push_str("VIDEO SETTINGS\n");
    for p in VIDEO_PROPS {
        o.push_str(&describe(p));
        o.push('\n');
    }

    o.push_str("\nSHAPES\n");
    for (name, kind) in SHAPES {
        o.push_str(&format!("\n{name}: {}\n", shape_blurb(name)));
        for p in schema::shape_props(*kind) {
            o.push_str(&describe(p));
            o.push('\n');
        }
    }

    o.push_str("\nEASINGS  (ease NAME on a keyframe; shapes the move that ends there)\n  ");
    o.push_str(&schema::EASINGS.iter().map(|e| e.0).collect::<Vec<_>>().join(", "));

    o.push_str("\n\nFONTS  (font: \"NAME\" on text)\n  ");
    o.push_str(&schema::FONT_NAMES.join(", "));

    o.push_str("\n\nCONSTANTS\n  width height fps duration (from video), pi, none (transparent)\n\nFUNCTIONS\n");
    for b in BUILTINS {
        o.push_str(&format!("  {}\n", b.doc));
    }
    o.push('\n');
    o
}

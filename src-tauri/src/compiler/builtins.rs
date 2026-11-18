//! Built-in functions callable from scripts, e.g. `rgb(255, 0, 128)` or
//! `linear(90, #ff0000, #0000ff)`.
//!
//! They run at compile time (constant folding) or, for relative keyframes,
//! when a keyframe is resolved, so the IR never contains calls. To add one:
//! add a row to [`BUILTINS`] and an arm to [`call`]. Semantic analysis checks
//! the name, the argument count and each argument's type before calling.

use super::value::{Gradient, GradientKind, Rgba, Ty, Value};

/// What arguments a built-in takes.
pub enum Params {
    /// Exactly these types, in order.
    Fixed(&'static [Ty]),
    /// The `fixed` types first, then at least `min_rest` values of type `rest`.
    Variadic { fixed: &'static [Ty], rest: Ty, min_rest: usize },
}

impl Params {
    /// The type expected for argument `i`, or `None` if `i` is past the end.
    pub fn expected(&self, i: usize) -> Option<Ty> {
        match self {
            Params::Fixed(tys) => tys.get(i).copied(),
            Params::Variadic { fixed, rest, .. } => Some(fixed.get(i).copied().unwrap_or(*rest)),
        }
    }

    pub fn accepts_count(&self, n: usize) -> bool {
        match self {
            Params::Fixed(tys) => n == tys.len(),
            Params::Variadic { fixed, min_rest, .. } => n >= fixed.len() + min_rest,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Params::Fixed(tys) => format!("{} argument(s)", tys.len()),
            Params::Variadic { fixed, min_rest, .. } => {
                format!("at least {} argument(s)", fixed.len() + min_rest)
            }
        }
    }
}

pub struct Builtin {
    pub name: &'static str,
    pub params: Params,
    /// Return type, so calls on not-yet-known values can still be type-checked.
    pub ret: Ty,
    pub doc: &'static str,
}

const N: Ty = Ty::Number;

pub const BUILTINS: &[Builtin] = &[
    Builtin { name: "rgb", params: Params::Fixed(&[N, N, N]), ret: Ty::Color, doc: "rgb(r, g, b): color from 0-255 channels" },
    Builtin { name: "rgba", params: Params::Fixed(&[N, N, N, N]), ret: Ty::Color, doc: "rgba(r, g, b, alpha): alpha is 0 (clear) to 1 (opaque)" },
    Builtin { name: "min", params: Params::Fixed(&[N, N]), ret: N, doc: "min(a, b)" },
    Builtin { name: "max", params: Params::Fixed(&[N, N]), ret: N, doc: "max(a, b)" },
    Builtin { name: "abs", params: Params::Fixed(&[N]), ret: N, doc: "abs(a)" },
    Builtin { name: "clamp", params: Params::Fixed(&[N, N, N]), ret: N, doc: "clamp(v, lo, hi)" },
    Builtin { name: "sin", params: Params::Fixed(&[N]), ret: N, doc: "sin(degrees)" },
    Builtin { name: "cos", params: Params::Fixed(&[N]), ret: N, doc: "cos(degrees)" },
    Builtin {
        name: "linear",
        params: Params::Variadic { fixed: &[N], rest: Ty::Color, min_rest: 2 },
        ret: Ty::Gradient,
        doc: "linear(angle, color, color, ...): angle in degrees, 0 = left to right, 90 = top to bottom",
    },
    Builtin {
        name: "radial",
        params: Params::Variadic { fixed: &[], rest: Ty::Color, min_rest: 2 },
        ret: Ty::Gradient,
        doc: "radial(color, color, ...): from the center outwards",
    },
];

pub fn find(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

fn num(v: &Value) -> Result<f64, String> {
    match v {
        Value::Number(n) => Ok(*n),
        other => Err(format!("expected a number argument, found a {}", other.ty())),
    }
}

fn color(v: &Value) -> Result<Rgba, String> {
    match v {
        Value::Color(c) => Ok(*c),
        other => Err(format!("expected a color argument, found a {}", other.ty())),
    }
}

pub fn call(name: &str, args: &[Value]) -> Result<Value, String> {
    let byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    Ok(match name {
        "linear" => {
            let angle = num(&args[0])?;
            let stops = args[1..].iter().map(color).collect::<Result<Vec<_>, _>>()?;
            Value::Gradient(Gradient { kind: GradientKind::Linear { angle }, stops })
        }
        "radial" => {
            let stops = args.iter().map(color).collect::<Result<Vec<_>, _>>()?;
            Value::Gradient(Gradient { kind: GradientKind::Radial, stops })
        }
        _ => {
            let n = args.iter().map(num).collect::<Result<Vec<_>, _>>()?;
            match name {
                "rgb" => Value::Color(Rgba::rgb(byte(n[0]), byte(n[1]), byte(n[2]))),
                "rgba" => Value::Color(Rgba::new(byte(n[0]), byte(n[1]), byte(n[2]), byte(n[3].clamp(0.0, 1.0) * 255.0))),
                "min" => Value::Number(n[0].min(n[1])),
                "max" => Value::Number(n[0].max(n[1])),
                "abs" => Value::Number(n[0].abs()),
                "clamp" => Value::Number(n[0].max(n[1]).min(n[2])),
                "sin" => Value::Number(n[0].to_radians().sin()),
                "cos" => Value::Number(n[0].to_radians().cos()),
                _ => return Err(format!("unknown function '{name}'")),
            }
        }
    })
}

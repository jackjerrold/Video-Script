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
    Builtin { name: "round", params: Params::Fixed(&[N]), ret: N, doc: "round(a): nearest whole number" },
    Builtin { name: "floor", params: Params::Fixed(&[N]), ret: N, doc: "floor(a): round down" },
    Builtin { name: "ceil", params: Params::Fixed(&[N]), ret: N, doc: "ceil(a): round up" },
    Builtin { name: "sqrt", params: Params::Fixed(&[N]), ret: N, doc: "sqrt(a): square root" },
    Builtin { name: "pow", params: Params::Fixed(&[N, N]), ret: N, doc: "pow(base, exponent)" },
    Builtin { name: "mod", params: Params::Fixed(&[N, N]), ret: N, doc: "mod(a, b): remainder, never negative for positive b" },
    Builtin { name: "lerp", params: Params::Fixed(&[N, N, N]), ret: N, doc: "lerp(a, b, t): a + (b - a) * t" },
    Builtin { name: "hsl", params: Params::Fixed(&[N, N, N]), ret: Ty::Color, doc: "hsl(hue 0-360, saturation 0-100, lightness 0-100)" },
    Builtin { name: "mix", params: Params::Fixed(&[Ty::Color, Ty::Color, N]), ret: Ty::Color, doc: "mix(color1, color2, t): blend two colors, t from 0 to 1" },
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

/// HSL (hue in degrees, saturation and lightness 0..100) to a color.
fn hsl(h: f64, s: f64, l: f64) -> Rgba {
    let (s, l) = ((s / 100.0).clamp(0.0, 1.0), (l / 100.0).clamp(0.0, 1.0));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgba::rgb(byte(r), byte(g), byte(b))
}

pub fn call(name: &str, args: &[Value]) -> Result<Value, String> {
    let byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    let out = match name {
        "linear" => {
            let angle = num(&args[0])?;
            let stops = args[1..].iter().map(color).collect::<Result<Vec<_>, _>>()?;
            Value::Gradient(Gradient { kind: GradientKind::Linear { angle }, stops })
        }
        "radial" => {
            let stops = args.iter().map(color).collect::<Result<Vec<_>, _>>()?;
            Value::Gradient(Gradient { kind: GradientKind::Radial, stops })
        }
        "mix" => Value::Color(color(&args[0])?.lerp(color(&args[1])?, num(&args[2])?.clamp(0.0, 1.0))),
        _ => {
            let n = args.iter().map(num).collect::<Result<Vec<_>, _>>()?;
            match name {
                "rgb" => Value::Color(Rgba::rgb(byte(n[0]), byte(n[1]), byte(n[2]))),
                "rgba" => Value::Color(Rgba::new(byte(n[0]), byte(n[1]), byte(n[2]), byte(n[3].clamp(0.0, 1.0) * 255.0))),
                "hsl" => Value::Color(hsl(n[0], n[1], n[2])),
                "min" => Value::Number(n[0].min(n[1])),
                "max" => Value::Number(n[0].max(n[1])),
                "abs" => Value::Number(n[0].abs()),
                "clamp" => Value::Number(n[0].max(n[1]).min(n[2])),
                "round" => Value::Number(n[0].round()),
                "floor" => Value::Number(n[0].floor()),
                "ceil" => Value::Number(n[0].ceil()),
                "sqrt" => Value::Number(n[0].sqrt()),
                "pow" => Value::Number(n[0].powf(n[1])),
                "mod" => Value::Number(n[0].rem_euclid(n[1])),
                "lerp" => Value::Number(n[0] + (n[1] - n[0]) * n[2]),
                "sin" => Value::Number(n[0].to_radians().sin()),
                "cos" => Value::Number(n[0].to_radians().cos()),
                _ => return Err(format!("unknown function '{name}'")),
            }
        }
    };
    // sqrt(-1), mod(x, 0) and friends produce NaN or infinity: report instead of propagating.
    if let Value::Number(n) = &out {
        if !n.is_finite() {
            return Err(format!("'{name}' produced a result that is not a finite number"));
        }
    }
    Ok(out)
}

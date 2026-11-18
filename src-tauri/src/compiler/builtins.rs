//! Built-in functions callable from scripts, e.g. `rgb(255, 0, 128)`.
//!
//! They run at compile time (constant folding), so the IR never contains
//! calls. To add one: add a row to [`BUILTINS`] and an arm to [`call`].
//! Semantic analysis checks the name and argument count before calling.

use super::value::{Rgb, Ty, Value};

pub struct Builtin {
    pub name: &'static str,
    pub arity: usize,
    /// Return type, so calls on not-yet-known values can still be type-checked.
    pub ret: Ty,
    pub doc: &'static str,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin { name: "rgb", arity: 3, ret: Ty::Color, doc: "rgb(r, g, b): color from 0-255 channels" },
    Builtin { name: "min", arity: 2, ret: Ty::Number, doc: "min(a, b)" },
    Builtin { name: "max", arity: 2, ret: Ty::Number, doc: "max(a, b)" },
    Builtin { name: "abs", arity: 1, ret: Ty::Number, doc: "abs(a)" },
    Builtin { name: "clamp", arity: 3, ret: Ty::Number, doc: "clamp(v, lo, hi)" },
    Builtin { name: "sin", arity: 1, ret: Ty::Number, doc: "sin(degrees)" },
    Builtin { name: "cos", arity: 1, ret: Ty::Number, doc: "cos(degrees)" },
];

pub fn find(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

fn nums(args: &[Value]) -> Result<Vec<f64>, String> {
    args.iter()
        .map(|a| match a {
            Value::Number(n) => Ok(*n),
            other => Err(format!("expected a number argument, found a {}", other.ty())),
        })
        .collect()
}

pub fn call(name: &str, args: &[Value]) -> Result<Value, String> {
    let n = nums(args)?;
    let byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    Ok(match name {
        "rgb" => Value::Color(Rgb(byte(n[0]), byte(n[1]), byte(n[2]))),
        "min" => Value::Number(n[0].min(n[1])),
        "max" => Value::Number(n[0].max(n[1])),
        "abs" => Value::Number(n[0].abs()),
        "clamp" => Value::Number(n[0].max(n[1]).min(n[2])),
        "sin" => Value::Number(n[0].to_radians().sin()),
        "cos" => Value::Number(n[0].to_radians().cos()),
        _ => return Err(format!("unknown function '{name}'")),
    })
}

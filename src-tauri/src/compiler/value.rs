//! Runtime values and their types.
//!
//! Everything in a script that is not structure (numbers, colors, strings)
//! is a [`Value`]. Adding a new kind of value (say, a 2D vector) means adding
//! a variant here, a [`Ty`], an arm in `lerp`, and a literal in the lexer.

use serde::{Serialize, Serializer};
use std::fmt;

/// An 8-bit RGB color.
#[derive(Clone, Copy, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Per-channel linear interpolation in sRGB space.
    pub fn lerp(self, other: Rgb, t: f64) -> Rgb {
        let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round().clamp(0.0, 255.0) as u8;
        Rgb(mix(self.0, other.0), mix(self.1, other.1), mix(self.2, other.2))
    }
}

impl fmt::Debug for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.hex())
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

/// The static type of a [`Value`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Number,
    Color,
    Str,
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Ty::Number => "number",
            Ty::Color => "color",
            Ty::Str => "string",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Color(Rgb),
    Str(String),
}

impl Value {
    pub fn ty(&self) -> Ty {
        match self {
            Value::Number(_) => Ty::Number,
            Value::Color(_) => Ty::Color,
            Value::Str(_) => Ty::Str,
        }
    }

    /// Interpolate towards `other` by `t` in `0..=1`. Values that cannot be
    /// blended (strings) hold until `t` reaches 1, then jump.
    pub fn lerp(&self, other: &Value, t: f64) -> Value {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a + (b - a) * t),
            (Value::Color(a), Value::Color(b)) => Value::Color(a.lerp(*b, t)),
            _ => {
                if t >= 1.0 {
                    other.clone()
                } else {
                    self.clone()
                }
            }
        }
    }
}

// In JSON: numbers are numbers, colors are "#rrggbb", strings are strings.
impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Number(n) => s.serialize_f64(*n),
            Value::Color(c) => c.serialize(s),
            Value::Str(v) => s.serialize_str(v),
        }
    }
}

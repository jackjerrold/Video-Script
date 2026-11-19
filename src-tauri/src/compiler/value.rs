//! Runtime values and their types.
//!
//! Everything in a script that is not structure (numbers, colors, gradients,
//! strings) is a [`Value`]. Adding a new kind of value means adding a variant
//! here, a [`Ty`], an arm in `lerp`, and (if it needs one) a literal in the lexer.

use serde::{Serialize, Serializer};
use std::fmt;

/// An 8-bit RGBA color (not premultiplied). Alpha 255 is opaque.
#[derive(Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const NONE: Rgba = Rgba::new(0, 0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 255 }
    }

    /// `#rrggbb`, or `#rrggbbaa` when not fully opaque.
    pub fn hex(&self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// Linear interpolation. Opaque colors blend per channel; when either
    /// color is translucent the blend is done in premultiplied space, so
    /// fading red to `none` stays red and just gets more transparent instead
    /// of drifting through dark red.
    pub fn lerp(self, o: Rgba, t: f64) -> Rgba {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        if self.a == 255 && o.a == 255 {
            let ch = |a: u8, b: u8| mix(a as f64, b as f64).round().clamp(0.0, 255.0) as u8;
            return Rgba::rgb(ch(self.r, o.r), ch(self.g, o.g), ch(self.b, o.b));
        }
        let (a1, a2) = (self.a as f64 / 255.0, o.a as f64 / 255.0);
        let a = mix(a1, a2);
        if a <= 0.0 {
            return Rgba::NONE;
        }
        let ch = |c1: u8, c2: u8| (mix(c1 as f64 * a1, c2 as f64 * a2) / a).round().clamp(0.0, 255.0) as u8;
        Rgba::new(ch(self.r, o.r), ch(self.g, o.g), ch(self.b, o.b), (a * 255.0).round() as u8)
    }
}

impl fmt::Debug for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.hex())
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum GradientKind {
    /// `angle` in degrees: 0 runs left to right, 90 top to bottom.
    Linear { angle: f64 },
    /// From the center of the shape out to its larger half-extent.
    Radial,
}

/// A gradient fill or stroke: a kind plus color stops spaced evenly from 0 to 1.
/// Its geometry is relative to the bounding box of whatever it paints, so it
/// moves, rotates and scales with the shape.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Gradient {
    #[serde(flatten)]
    pub kind: GradientKind,
    pub stops: Vec<Rgba>,
}

impl Gradient {
    /// A gradient of the same shape whose stops are all `c` (used to blend a
    /// plain color into a gradient).
    fn flat_like(&self, c: Rgba) -> Gradient {
        Gradient { kind: self.kind.clone(), stops: vec![c; self.stops.len()] }
    }

    /// Blend towards `o` when they have the same kind and stop count.
    fn lerp(&self, o: &Gradient, t: f64) -> Option<Gradient> {
        if self.stops.len() != o.stops.len() {
            return None;
        }
        let kind = match (&self.kind, &o.kind) {
            (GradientKind::Linear { angle: a }, GradientKind::Linear { angle: b }) => {
                GradientKind::Linear { angle: a + (b - a) * t }
            }
            (GradientKind::Radial, GradientKind::Radial) => GradientKind::Radial,
            _ => return None,
        };
        let stops = self.stops.iter().zip(&o.stops).map(|(a, b)| a.lerp(*b, t)).collect();
        Some(Gradient { kind, stops })
    }
}

/// The static type of a [`Value`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Number,
    Color,
    Gradient,
    /// "A color or a gradient": what `fill` and `stroke` accept.
    Paint,
    Str,
}

impl Ty {
    /// Can a value of type `other` be used where `self` is expected?
    pub fn accepts(self, other: Ty) -> bool {
        self == other || (self == Ty::Paint && matches!(other, Ty::Color | Ty::Gradient))
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Ty::Number => "number",
            Ty::Color => "color",
            Ty::Gradient => "gradient",
            Ty::Paint => "color or gradient",
            Ty::Str => "string",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Color(Rgba),
    Gradient(Gradient),
    Str(String),
}

impl Value {
    pub fn ty(&self) -> Ty {
        match self {
            Value::Number(_) => Ty::Number,
            Value::Color(_) => Ty::Color,
            Value::Gradient(_) => Ty::Gradient,
            Value::Str(_) => Ty::Str,
        }
    }

    /// Interpolate towards `other` by `t` in `0..=1`. Values that cannot be
    /// blended (strings, mismatched gradients) hold until `t` reaches 1, then jump.
    pub fn lerp(&self, other: &Value, t: f64) -> Value {
        // Easings like `back` and `elastic` overshoot 0..1. That is fine for
        // numbers (it is the point), but colors would go out of gamut.
        let tc = t.clamp(0.0, 1.0);
        let blended = match (self, other) {
            (Value::Number(a), Value::Number(b)) => Some(Value::Number(a + (b - a) * t)),
            (Value::Color(a), Value::Color(b)) => Some(Value::Color(a.lerp(*b, tc))),
            (Value::Gradient(a), Value::Gradient(b)) => a.lerp(b, tc).map(Value::Gradient),
            (Value::Color(c), Value::Gradient(g)) => g.flat_like(*c).lerp(g, tc).map(Value::Gradient),
            (Value::Gradient(g), Value::Color(c)) => g.lerp(&g.flat_like(*c), tc).map(Value::Gradient),
            _ => None,
        };
        blended.unwrap_or_else(|| if t >= 1.0 { other.clone() } else { self.clone() })
    }
}

// In JSON: numbers are numbers, colors are "#rrggbb[aa]", gradients are objects,
// strings are strings.
impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Number(n) => s.serialize_f64(*n),
            Value::Color(c) => c.serialize(s),
            Value::Gradient(g) => g.serialize(s),
            Value::Str(v) => s.serialize_str(v),
        }
    }
}

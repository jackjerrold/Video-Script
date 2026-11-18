//! The language's vocabulary, in one place.
//!
//! Which shapes exist, which properties each has (and their types, defaults
//! and sensible ranges), which easings exist, and the `video` settings are all
//! *data* here, not code scattered through the parser. Semantic analysis reads
//! these tables, so most language extensions start (and often end) in this file.

use super::ir::Easing;
use super::value::{Rgb, Ty, Value};
use serde::Serialize;

// ---- shapes ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeKind {
    Rect,
    Circle,
    Text,
    /// A container: has a transform but draws nothing itself.
    Group,
}

impl ShapeKind {
    pub fn name(self) -> &'static str {
        SHAPES.iter().find(|(_, k)| *k == self).map(|(n, _)| *n).unwrap_or("?")
    }
}

/// Name used in scripts -> kind. Add a line here for each new shape.
pub const SHAPES: &[(&str, ShapeKind)] = &[
    ("rect", ShapeKind::Rect),
    ("circle", ShapeKind::Circle),
    ("text", ShapeKind::Text),
    ("group", ShapeKind::Group),
];

pub fn find_shape(name: &str) -> Option<ShapeKind> {
    SHAPES.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

/// True if any node kind has a property with this name.
pub fn is_prop_name(name: &str) -> bool {
    SHAPES.iter().any(|(_, k)| find_prop(*k, name).is_some())
}

// ---- properties --------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub enum PropDefault {
    Num(f64),
    Color(Rgb),
    Str(&'static str),
}

impl PropDefault {
    pub fn value(self) -> Value {
        match self {
            PropDefault::Num(n) => Value::Number(n),
            PropDefault::Color(c) => Value::Color(c),
            PropDefault::Str(s) => Value::Str(s.to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PropDef {
    pub name: &'static str,
    pub ty: Ty,
    pub default: PropDefault,
    /// Inclusive (min, max) for numbers. Out of range is a warning on shapes
    /// and a hard error on `video` settings.
    pub range: Option<(f64, f64)>,
}

const fn num(name: &'static str, default: f64) -> PropDef {
    PropDef { name, ty: Ty::Number, default: PropDefault::Num(default), range: None }
}

const fn num_in(name: &'static str, default: f64, lo: f64, hi: f64) -> PropDef {
    PropDef { name, ty: Ty::Number, default: PropDefault::Num(default), range: Some((lo, hi)) }
}

const fn color(name: &'static str, default: Rgb) -> PropDef {
    PropDef { name, ty: Ty::Color, default: PropDefault::Color(default), range: None }
}

const fn string(name: &'static str, default: &'static str) -> PropDef {
    PropDef { name, ty: Ty::Str, default: PropDefault::Str(default), range: None }
}

/// Settings accepted inside `video { ... }`.
pub const VIDEO_PROPS: &[PropDef] = &[
    num_in("width", 640.0, 1.0, 4096.0),
    num_in("height", 360.0, 1.0, 4096.0),
    num_in("fps", 30.0, 1.0, 240.0),
    num_in("duration", 4.0, 0.001, 3600.0),
    color("background", Rgb(20, 20, 28)),
];

/// Properties every node has, groups included. A group's transform applies
/// to all of its children, and its opacity multiplies theirs.
const TRANSFORM: &[PropDef] = &[
    num("x", 0.0),
    num("y", 0.0),
    num("rot", 0.0),
    num("scale", 1.0),
    num_in("opacity", 1.0, 0.0, 1.0),
];

/// Properties of anything that is actually drawn.
const PAINT: &[PropDef] = &[color("fill", Rgb(255, 255, 255))];

const RECT: &[PropDef] = &[num_in("w", 100.0, 0.0, f64::INFINITY), num_in("h", 100.0, 0.0, f64::INFINITY)];

const CIRCLE: &[PropDef] = &[num_in("r", 50.0, 0.0, f64::INFINITY)];

const TEXT: &[PropDef] = &[string("text", ""), num_in("size", 32.0, 0.0, f64::INFINITY)];

const NONE: &[PropDef] = &[];

/// All properties of a node kind: transform, then paint, then the kind's own.
pub fn shape_props(kind: ShapeKind) -> impl Iterator<Item = &'static PropDef> {
    let (paint, extra): (&'static [PropDef], &'static [PropDef]) = match kind {
        ShapeKind::Rect => (PAINT, RECT),
        ShapeKind::Circle => (PAINT, CIRCLE),
        ShapeKind::Text => (PAINT, TEXT),
        ShapeKind::Group => (NONE, NONE),
    };
    TRANSFORM.iter().chain(paint.iter()).chain(extra.iter())
}

pub fn find_prop(kind: ShapeKind, name: &str) -> Option<&'static PropDef> {
    shape_props(kind).find(|p| p.name == name)
}

// ---- easings -----------------------------------------------------------

pub const EASINGS: &[(&str, Easing)] = &[
    ("linear", Easing::Linear),
    ("in", Easing::In),
    ("out", Easing::Out),
    ("in_out", Easing::InOut),
    ("step", Easing::Step),
];

pub fn find_easing(name: &str) -> Option<Easing> {
    EASINGS.iter().find(|(n, _)| *n == name).map(|(_, e)| *e)
}

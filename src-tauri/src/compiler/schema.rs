//! The language's vocabulary, in one place.
//!
//! Which shapes exist, which properties each has (and their types, defaults
//! and sensible ranges), which easings exist, and the `video` settings are all
//! *data* here, not code scattered through the parser. Semantic analysis reads
//! these tables, so most language extensions start (and often end) in this file.

use super::ir::Easing;
use super::value::{Rgba, Ty, Value};
use serde::Serialize;

// ---- shapes ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeKind {
    Rect,
    Circle,
    Ellipse,
    /// A straight segment from its own origin to an offset (`x2`, `y2`).
    Line,
    /// A regular polygon (triangle, hexagon, ...).
    Polygon,
    Star,
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
    ("ellipse", ShapeKind::Ellipse),
    ("line", ShapeKind::Line),
    ("polygon", ShapeKind::Polygon),
    ("star", ShapeKind::Star),
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
    Color(Rgba),
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
    /// For strings: the only accepted values (an error otherwise).
    pub choices: Option<&'static [&'static str]>,
}

const fn num(name: &'static str, default: f64) -> PropDef {
    PropDef { name, ty: Ty::Number, default: PropDefault::Num(default), range: None, choices: None }
}

const fn num_in(name: &'static str, default: f64, lo: f64, hi: f64) -> PropDef {
    PropDef { name, ty: Ty::Number, default: PropDefault::Num(default), range: Some((lo, hi)), choices: None }
}

const fn color(name: &'static str, default: Rgba) -> PropDef {
    PropDef { name, ty: Ty::Color, default: PropDefault::Color(default), range: None, choices: None }
}

/// A color *or* gradient (for `fill` and `stroke`). The default must be a plain color.
const fn paint(name: &'static str, default: Rgba) -> PropDef {
    PropDef { name, ty: Ty::Paint, default: PropDefault::Color(default), range: None, choices: None }
}

const fn string(name: &'static str, default: &'static str) -> PropDef {
    PropDef { name, ty: Ty::Str, default: PropDefault::Str(default), range: None, choices: None }
}

/// A string that must be one of `choices`.
const fn choice(name: &'static str, default: &'static str, choices: &'static [&'static str]) -> PropDef {
    PropDef { name, ty: Ty::Str, default: PropDefault::Str(default), range: None, choices: Some(choices) }
}

/// Settings accepted inside `video { ... }`.
pub const VIDEO_PROPS: &[PropDef] = &[
    num_in("width", 640.0, 1.0, 4096.0),
    num_in("height", 360.0, 1.0, 4096.0),
    num_in("fps", 30.0, 1.0, 240.0),
    num_in("duration", 4.0, 0.001, 3600.0),
    color("background", Rgba::rgb(20, 20, 28)),
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

/// Properties of anything that is actually drawn. `fill` and `stroke` take a
/// color or a gradient; `none` (fully transparent) turns either off. The
/// outline is only drawn when `stroke_width` is above 0 and is centered on
/// the shape's edge.
const PAINT: &[PropDef] = &[
    paint("fill", Rgba::rgb(255, 255, 255)),
    paint("stroke", Rgba::rgb(255, 255, 255)),
    num_in("stroke_width", 0.0, 0.0, f64::INFINITY),
];

/// Effects every drawn shape has. `blur` is a gaussian blur radius in pixels
/// (it scales with the shape's transform).
const EFFECTS: &[PropDef] = &[num_in("blur", 0.0, 0.0, f64::INFINITY)];

/// Lines are all stroke: no fill, and a visible default width.
const PAINT_LINE: &[PropDef] =
    &[paint("stroke", Rgba::rgb(255, 255, 255)), num_in("stroke_width", 4.0, 0.0, f64::INFINITY)];

const RECT: &[PropDef] = &[
    num_in("w", 100.0, 0.0, f64::INFINITY),
    num_in("h", 100.0, 0.0, f64::INFINITY),
    num_in("radius", 0.0, 0.0, f64::INFINITY),
];

const CIRCLE: &[PropDef] = &[num_in("r", 50.0, 0.0, f64::INFINITY)];

const ELLIPSE: &[PropDef] = &[num_in("rx", 60.0, 0.0, f64::INFINITY), num_in("ry", 40.0, 0.0, f64::INFINITY)];

const LINE: &[PropDef] = &[num("x2", 100.0), num("y2", 0.0)];

/// `sides` is rounded to a whole number when drawn.
const POLYGON: &[PropDef] = &[num_in("r", 50.0, 0.0, f64::INFINITY), num_in("sides", 6.0, 3.0, 64.0)];

/// `inner` is the inner radius as a fraction of `r` (0 to 1).
const STAR: &[PropDef] = &[
    num_in("r", 50.0, 0.0, f64::INFINITY),
    num_in("points", 5.0, 2.0, 64.0),
    num_in("inner", 0.5, 0.0, 1.0),
];

/// Bundled fonts. The data lives in `backend/text.rs`; a test checks that
/// every name here has one. Add a name here and a row there.
pub const FONT_NAMES: &[&str] = &["sans", "bold", "serif", "mono", "display"];

pub const ALIGNS: &[&str] = &["left", "center", "right"];

/// `max_width` 0 means "do not wrap". `line_height` multiplies the font's natural line spacing.
const TEXT: &[PropDef] = &[
    string("text", ""),
    choice("font", "sans", FONT_NAMES),
    num_in("size", 32.0, 0.0, f64::INFINITY),
    choice("align", "center", ALIGNS),
    num_in("max_width", 0.0, 0.0, f64::INFINITY),
    num_in("line_height", 1.0, 0.0, f64::INFINITY),
];

const NONE: &[PropDef] = &[];

/// All properties of a node kind: transform, then paint, then the kind's own, then effects.
pub fn shape_props(kind: ShapeKind) -> impl Iterator<Item = &'static PropDef> {
    let (paint, extra, fx): (&'static [PropDef], &'static [PropDef], &'static [PropDef]) = match kind {
        ShapeKind::Rect => (PAINT, RECT, EFFECTS),
        ShapeKind::Circle => (PAINT, CIRCLE, EFFECTS),
        ShapeKind::Ellipse => (PAINT, ELLIPSE, EFFECTS),
        ShapeKind::Line => (PAINT_LINE, LINE, EFFECTS),
        ShapeKind::Polygon => (PAINT, POLYGON, EFFECTS),
        ShapeKind::Star => (PAINT, STAR, EFFECTS),
        ShapeKind::Text => (PAINT, TEXT, EFFECTS),
        ShapeKind::Group => (NONE, NONE, NONE),
    };
    TRANSFORM.iter().chain(paint.iter()).chain(extra.iter()).chain(fx.iter())
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
    ("smooth", Easing::Smooth),
    ("back", Easing::Back),
    ("bounce", Easing::Bounce),
    ("elastic", Easing::Elastic),
    ("step", Easing::Step),
];

pub fn find_easing(name: &str) -> Option<Easing> {
    EASINGS.iter().find(|(n, _)| *n == name).map(|(_, e)| *e)
}

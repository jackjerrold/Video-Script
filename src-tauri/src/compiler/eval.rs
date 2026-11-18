//! Stage 6a: evaluating the IR at a point in time.
//!
//! `eval(program, t)` turns the timeline-based IR into a [`Frame`]: a flat
//! "display list" of drawable shapes with every property resolved to a
//! concrete value. Groups disappear here: their transforms and opacity are
//! folded into each descendant's `world` transform and `opacity`.
//!
//! Backends (the rasterizer today; SVG, GPU, ... tomorrow) only ever see
//! frames, never keyframes, easing or groups.

use super::geom::Affine;
use super::ir::{Program, Prop, Shape};
use super::schema::ShapeKind;
use super::value::{Rgba, Value};

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub background: Rgba,
    /// In drawing order (parents' children in order, depth first).
    pub shapes: Vec<DrawShape>,
}

pub struct DrawShape {
    pub kind: ShapeKind,
    /// The shape's own properties at this time (including x, y, rot, ...).
    pub props: Vec<(&'static str, Value)>,
    /// Local-to-canvas transform: every ancestor group's transform, then this shape's.
    pub world: Affine,
    /// This shape's opacity times all its ancestors', clamped to 0..=1.
    pub opacity: f64,
}

impl DrawShape {
    pub fn num(&self, name: &str) -> f64 {
        match self.props.iter().find(|(n, _)| *n == name) {
            Some((_, Value::Number(n))) => *n,
            _ => 0.0,
        }
    }

    pub fn color(&self, name: &str) -> Rgba {
        match self.props.iter().find(|(n, _)| *n == name) {
            Some((_, Value::Color(c))) => *c,
            _ => Rgba::rgb(255, 255, 255),
        }
    }

    /// Any property's resolved value (used for paints, which may be a color or a gradient).
    pub fn value(&self, name: &str) -> Option<&Value> {
        self.props.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    pub fn text(&self, name: &str) -> &str {
        match self.props.iter().find(|(n, _)| *n == name) {
            Some((_, Value::Str(s))) => s,
            _ => "",
        }
    }
}

pub fn eval(program: &Program, t: f64) -> Frame {
    let mut shapes = Vec::new();
    for s in &program.shapes {
        walk(s, t, Affine::IDENTITY, 1.0, &mut shapes);
    }
    Frame {
        width: program.video.width,
        height: program.video.height,
        background: program.video.background,
        shapes,
    }
}

fn walk(s: &Shape, t: f64, parent: Affine, parent_opacity: f64, out: &mut Vec<DrawShape>) {
    let props: Vec<(&'static str, Value)> = s.props.iter().map(|p| (p.name, eval_prop(p, t))).collect();
    let n = |name: &str| match props.iter().find(|(k, _)| *k == name) {
        Some((_, Value::Number(v))) => *v,
        _ => 0.0,
    };

    let world = parent.then(Affine::local(n("x"), n("y"), n("rot"), n("scale")));
    let opacity = parent_opacity * n("opacity").clamp(0.0, 1.0);

    if s.kind == ShapeKind::Group {
        for child in &s.children {
            walk(child, t, world, opacity, out);
        }
    } else {
        out.push(DrawShape { kind: s.kind, props, world, opacity });
    }
}

/// Value of one property at time `t`: hold before the first key and after the
/// last; in between, interpolate using the easing of the key being approached.
pub fn eval_prop(prop: &Prop, t: f64) -> Value {
    let keys = &prop.keys;
    if keys.is_empty() {
        return prop.base.clone();
    }
    if t <= keys[0].time {
        return keys[0].value.clone();
    }
    for w in keys.windows(2) {
        if t < w[1].time {
            let (a, b) = (&w[0], &w[1]);
            let u = (t - a.time) / (b.time - a.time);
            return a.value.lerp(&b.value, b.ease.apply(u));
        }
    }
    keys.last().unwrap().value.clone()
}

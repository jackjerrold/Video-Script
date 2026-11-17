//! Stage 6a: evaluating the IR at a point in time.
//!
//! `eval(program, t)` turns the timeline-based IR into a [`Frame`]: a flat
//! "display list" of shapes with every property resolved to a concrete value.
//! Backends (the rasterizer today; SVG, GPU, ... tomorrow) only ever see
//! frames, never keyframes or easing.

use super::ir::{Program, Prop};
use super::schema::ShapeKind;
use super::value::{Rgb, Value};

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub background: Rgb,
    pub shapes: Vec<DrawShape>,
}

pub struct DrawShape {
    pub kind: ShapeKind,
    pub props: Vec<(&'static str, Value)>,
}

impl DrawShape {
    pub fn num(&self, name: &str) -> f64 {
        match self.props.iter().find(|(n, _)| *n == name) {
            Some((_, Value::Number(n))) => *n,
            _ => 0.0,
        }
    }

    pub fn color(&self, name: &str) -> Rgb {
        match self.props.iter().find(|(n, _)| *n == name) {
            Some((_, Value::Color(c))) => *c,
            _ => Rgb(255, 255, 255),
        }
    }
}

pub fn eval(program: &Program, t: f64) -> Frame {
    Frame {
        width: program.video.width,
        height: program.video.height,
        background: program.video.background,
        shapes: program
            .shapes
            .iter()
            .map(|s| DrawShape {
                kind: s.kind,
                props: s.props.iter().map(|p| (p.name, eval_prop(p, t))).collect(),
            })
            .collect(),
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

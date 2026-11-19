//! Stage 4 output: the intermediate representation (IR).
//!
//! The IR is what is left after semantic analysis: no variables, no
//! expressions, no names to resolve. Every value is already computed. It is
//! deliberately plain data that derives `Serialize`, so it can be dumped to
//! JSON, tested, optimized, or consumed by any backend.
//!
//! Each shape owns a list of [`Prop`]s (one per property in its schema), and
//! each prop owns a sorted timeline of [`Key`]s.

use super::schema::ShapeKind;
use super::value::{Rgba, Value};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    In,
    Out,
    InOut,
    /// Smoothstep: gentle at both ends.
    Smooth,
    /// Overshoots the target slightly, then settles.
    Back,
    /// Lands on the target and bounces.
    Bounce,
    /// Springs past the target and wobbles to rest.
    Elastic,
    Step,
}

impl Easing {
    /// Map linear progress `t` (0..=1) to eased progress.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::In => t * t,
            Easing::Out => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::InOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Easing::Smooth => t * t * (3.0 - 2.0 * t),
            Easing::Back => {
                let (c1, c3) = (1.70158, 2.70158);
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
            Easing::Bounce => {
                let (n1, d1) = (7.5625, 2.75);
                if t < 1.0 / d1 {
                    n1 * t * t
                } else if t < 2.0 / d1 {
                    let t = t - 1.5 / d1;
                    n1 * t * t + 0.75
                } else if t < 2.5 / d1 {
                    let t = t - 2.25 / d1;
                    n1 * t * t + 0.9375
                } else {
                    let t = t - 2.625 / d1;
                    n1 * t * t + 0.984375
                }
            }
            Easing::Elastic => {
                if t <= 0.0 || t >= 1.0 {
                    t
                } else {
                    2f64.powf(-10.0 * t) * ((t * 10.0 - 0.75) * (2.0 * std::f64::consts::PI / 3.0)).sin() + 1.0
                }
            }
            Easing::Step => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Program {
    pub video: Video,
    pub shapes: Vec<Shape>,
    /// Inspector parameters (`@slider ...`), in source order.
    pub params: Vec<super::params::Param>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Video {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration: f64,
    pub background: Rgba,
}

impl Program {
    /// Find a node anywhere in the tree by name.
    pub fn find(&self, name: &str) -> Option<&Shape> {
        self.shapes.iter().find_map(|s| s.find(name))
    }
}

impl Video {
    pub fn frame_count(&self) -> u32 {
        ((self.duration * self.fps as f64).round() as u32).max(1)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Shape {
    pub name: String,
    pub kind: ShapeKind,
    pub props: Vec<Prop>,
    /// Non-empty only for groups. Children are drawn in order, after the parent's transform.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Shape>,
}

impl Shape {
    pub fn find(&self, name: &str) -> Option<&Shape> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }
}

/// One animatable property of one shape.
///
/// If `keys` is empty the property is constant and equals `base`. Otherwise
/// `keys` is sorted by time and fully describes the value over time; if the
/// user's first keyframe was after t=0, lowering inserted a synthetic key at
/// t=0 holding `base`, so evaluation never needs a special case.
#[derive(Clone, Debug, Serialize)]
pub struct Prop {
    pub name: &'static str,
    pub base: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<Key>,
}

/// A keyframe. `ease` shapes the segment that *ends* at this key.
#[derive(Clone, Debug, Serialize)]
pub struct Key {
    pub time: f64,
    pub value: Value,
    pub ease: Easing,
}

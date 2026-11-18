//! Raster backend: draws a frame into a pixel buffer with `tiny-skia`.
//!
//! This is the only place that knows how a shape *looks*. To support a new
//! shape, add a match arm in [`shape_path`]. Positioning, rotation, scaling,
//! group nesting and opacity are already resolved into `world` and `opacity`
//! by the evaluator, so shapes are described around their own origin.
//!
//! Each shape is drawn in two passes: fill, then stroke. Both take a color or
//! a gradient. Gradients are defined relative to the shape's bounding box in
//! its own coordinates, so they follow the shape's transform.

use super::text::{self, Align, TextStyle};
use crate::compiler::eval::{DrawShape, Frame};
use crate::compiler::schema::ShapeKind;
use crate::compiler::value::{Gradient as KfGradient, GradientKind, Value};
use tiny_skia::*;

pub fn draw(frame: &Frame) -> Pixmap {
    let mut pm = Pixmap::new(frame.width, frame.height).expect("size is validated by the compiler");
    let bg = frame.background; // background transparency is ignored: video frames are opaque
    pm.fill(Color::from_rgba8(bg.r, bg.g, bg.b, 255));

    for s in &frame.shapes {
        if s.opacity <= 0.0 {
            continue;
        }
        let Some(path) = shape_path(s) else { continue };
        let bounds = path.bounds();
        let w = &s.world;
        let tf = Transform::from_row(w.a as f32, w.b as f32, w.c as f32, w.d as f32, w.e as f32, w.f as f32);

        if let Some(paint) = s.value("fill").and_then(|v| make_paint(v, s.opacity, &bounds)) {
            pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
        }

        let width = s.num("stroke_width") as f32;
        if width > 0.0 {
            if let Some(paint) = s.value("stroke").and_then(|v| make_paint(v, s.opacity, &bounds)) {
                let stroke = Stroke {
                    width,
                    // Mitered corners keep rectangles crisp; glyph outlines look better rounded.
                    line_join: if s.kind == ShapeKind::Text { LineJoin::Round } else { LineJoin::Miter },
                    ..Stroke::default()
                };
                pm.stroke_path(&path, &paint, &stroke, tf, None);
            }
        }
    }
    pm
}

/// The shape's outline, centered on the origin. `None` skips drawing
/// (zero or negative sizes can occur mid-animation; they must not panic).
fn shape_path(s: &DrawShape) -> Option<Path> {
    match s.kind {
        ShapeKind::Rect => {
            let (w, h) = (s.num("w") as f32, s.num("h") as f32);
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            Rect::from_xywh(-w / 2.0, -h / 2.0, w, h).map(PathBuilder::from_rect)
        }
        ShapeKind::Circle => {
            let r = s.num("r") as f32;
            if r <= 0.0 {
                return None;
            }
            PathBuilder::from_circle(0.0, 0.0, r)
        }
        ShapeKind::Text => text::text_path(&TextStyle {
            text: s.text("text"),
            size: s.num("size") as f32,
            align: Align::parse(s.text("align")),
            max_width: s.num("max_width") as f32,
            line_height: s.num("line_height") as f32,
        }),
        // Groups are flattened away by the evaluator and never reach a backend.
        ShapeKind::Group => None,
    }
}

/// A `tiny-skia` paint for a color or gradient value, with `opacity` folded
/// into its alpha. `None` means "draw nothing" (fully transparent or not a paint).
fn make_paint(v: &Value, opacity: f64, bounds: &Rect) -> Option<Paint<'static>> {
    let mut paint = Paint::default();
    paint.anti_alias = true;
    match v {
        Value::Color(c) => {
            let a = scaled_alpha(c.a, opacity);
            if a == 0 {
                return None;
            }
            paint.set_color_rgba8(c.r, c.g, c.b, a);
        }
        Value::Gradient(g) => paint.shader = gradient_shader(g, bounds, opacity)?,
        _ => return None,
    }
    Some(paint)
}

fn scaled_alpha(a: u8, opacity: f64) -> u8 {
    (a as f64 * opacity).round().clamp(0.0, 255.0) as u8
}

/// Build the shader for a gradient over `bounds`.
///
/// Linear: the gradient line passes through the center of the box in the given
/// direction and is as long as the box's extent along that direction, so the
/// first and last stops land exactly on the box's far edges or corners.
/// Radial: centered on the box, reaching the last stop at its larger half-extent.
fn gradient_shader(g: &KfGradient, bounds: &Rect, opacity: f64) -> Option<Shader<'static>> {
    let n = g.stops.len();
    let color_of = |i: usize| {
        let c = g.stops[i];
        Color::from_rgba8(c.r, c.g, c.b, scaled_alpha(c.a, opacity))
    };
    match n {
        0 => return None,
        1 => return Some(Shader::SolidColor(color_of(0))),
        _ => {}
    }
    let stops: Vec<GradientStop> = (0..n).map(|i| GradientStop::new(i as f32 / (n - 1) as f32, color_of(i))).collect();
    let center = Point::from_xy(bounds.left() + bounds.width() / 2.0, bounds.top() + bounds.height() / 2.0);

    match g.kind {
        GradientKind::Linear { angle } => {
            let (sin, cos) = (angle as f32).to_radians().sin_cos();
            let half = bounds.width() / 2.0 * cos.abs() + bounds.height() / 2.0 * sin.abs();
            if half < 1e-4 {
                return Some(Shader::SolidColor(color_of(0)));
            }
            let (dx, dy) = (cos * half, sin * half);
            LinearGradient::new(
                Point::from_xy(center.x - dx, center.y - dy),
                Point::from_xy(center.x + dx, center.y + dy),
                stops,
                SpreadMode::Pad,
                Transform::identity(),
            )
        }
        GradientKind::Radial => {
            let radius = bounds.width().max(bounds.height()) / 2.0;
            if radius < 1e-4 {
                return Some(Shader::SolidColor(color_of(0)));
            }
            RadialGradient::new(center, 0.0, center, radius, stops, SpreadMode::Pad, Transform::identity())
        }
    }
}

pub fn to_png(pm: &Pixmap) -> Result<Vec<u8>, String> {
    pm.encode_png().map_err(|e| e.to_string())
}

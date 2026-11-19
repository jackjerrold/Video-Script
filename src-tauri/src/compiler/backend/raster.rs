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
//!
//! A shape with `blur` above 0 is drawn into its own transparent layer, which
//! is blurred and then composited onto the frame.

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
        let w = &s.world;
        let tf = Transform::from_row(w.a as f32, w.b as f32, w.c as f32, w.d as f32, w.e as f32, w.f as f32);

        // Blur is specified in the shape's own units, so it scales with the shape.
        let scale = (w.a * w.d - w.b * w.c).abs().sqrt();
        let sigma = (s.num("blur") * scale) as f32;
        if sigma >= 0.3 {
            if let Some(mut layer) = Pixmap::new(frame.width, frame.height) {
                paint_shape(&mut layer, s, &path, tf);
                gaussian_blur(&mut layer, sigma);
                pm.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
        } else {
            paint_shape(&mut pm, s, &path, tf);
        }
    }
    pm
}

/// Fill, then stroke.
fn paint_shape(target: &mut Pixmap, s: &DrawShape, path: &Path, tf: Transform) {
    let bounds = path.bounds();

    if let Some(paint) = s.value("fill").and_then(|v| make_paint(v, s.opacity, &bounds)) {
        target.fill_path(path, &paint, FillRule::Winding, tf, None);
    }

    let width = s.num("stroke_width") as f32;
    if width > 0.0 {
        if let Some(paint) = s.value("stroke").and_then(|v| make_paint(v, s.opacity, &bounds)) {
            let stroke = Stroke {
                width,
                // Rounded joins suit glyph outlines; rounded caps make lines look finished.
                line_join: if s.kind == ShapeKind::Text { LineJoin::Round } else { LineJoin::Miter },
                line_cap: if s.kind == ShapeKind::Line { LineCap::Round } else { LineCap::Butt },
                ..Stroke::default()
            };
            target.stroke_path(path, &paint, &stroke, tf, None);
        }
    }
}

/// The shape's outline, centered on the origin (a line starts there). `None`
/// skips drawing: zero or negative sizes occur mid-animation and must not panic.
fn shape_path(s: &DrawShape) -> Option<Path> {
    match s.kind {
        ShapeKind::Rect => {
            let (w, h) = (s.num("w") as f32, s.num("h") as f32);
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            let r = (s.num("radius") as f32).clamp(0.0, w.min(h) / 2.0);
            if r < 0.01 {
                Rect::from_xywh(-w / 2.0, -h / 2.0, w, h).map(PathBuilder::from_rect)
            } else {
                rounded_rect(w, h, r)
            }
        }
        ShapeKind::Circle => {
            let r = s.num("r") as f32;
            if r <= 0.0 {
                return None;
            }
            PathBuilder::from_circle(0.0, 0.0, r)
        }
        ShapeKind::Ellipse => {
            let (rx, ry) = (s.num("rx") as f32, s.num("ry") as f32);
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            Rect::from_xywh(-rx, -ry, rx * 2.0, ry * 2.0).and_then(PathBuilder::from_oval)
        }
        ShapeKind::Line => {
            let (x2, y2) = (s.num("x2") as f32, s.num("y2") as f32);
            let mut pb = PathBuilder::new();
            pb.move_to(0.0, 0.0);
            pb.line_to(x2, y2);
            pb.finish()
        }
        ShapeKind::Polygon => {
            let r = s.num("r") as f32;
            let n = (s.num("sides").round() as i32).clamp(3, 64) as usize;
            if r <= 0.0 {
                return None;
            }
            star_path(n, r, r) // equal radii: a regular polygon
        }
        ShapeKind::Star => {
            let r = s.num("r") as f32;
            let n = (s.num("points").round() as i32).clamp(2, 64) as usize;
            if r <= 0.0 {
                return None;
            }
            star_path(n, r, r * (s.num("inner") as f32).clamp(0.0, 1.0))
        }
        ShapeKind::Text => text::text_path(&TextStyle {
            text: s.text("text"),
            font: s.text("font"),
            size: s.num("size") as f32,
            align: Align::parse(s.text("align")),
            max_width: s.num("max_width") as f32,
            line_height: s.num("line_height") as f32,
        }),
        // Groups are flattened away by the evaluator and never reach a backend.
        ShapeKind::Group => None,
    }
}

/// `n` outer points on radius `outer`, alternating with `n` inner points on
/// radius `inner` (equal radii give a regular polygon). The first point is straight up.
fn star_path(n: usize, outer: f32, inner: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let vertices = if (outer - inner).abs() < f32::EPSILON { n } else { n * 2 };
    for i in 0..vertices {
        let radius = if vertices == n || i % 2 == 0 { outer } else { inner };
        let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / vertices as f32;
        let (x, y) = (radius * angle.cos(), radius * angle.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
    pb.finish()
}

/// A `w` by `h` rectangle centered on the origin with corner radius `r`
/// (corners are cubic approximations of quarter circles).
fn rounded_rect(w: f32, h: f32, r: f32) -> Option<Path> {
    let (l, t, rt, b) = (-w / 2.0, -h / 2.0, w / 2.0, h / 2.0);
    let k = r * 0.552_284_75; // control point offset for a quarter circle
    let mut pb = PathBuilder::new();
    pb.move_to(l + r, t);
    pb.line_to(rt - r, t);
    pb.cubic_to(rt - r + k, t, rt, t + r - k, rt, t + r);
    pb.line_to(rt, b - r);
    pb.cubic_to(rt, b - r + k, rt - r + k, b, rt - r, b);
    pb.line_to(l + r, b);
    pb.cubic_to(l + r - k, b, l, b - r + k, l, b - r);
    pb.line_to(l, t + r);
    pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
    pb.close();
    pb.finish()
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

// ---- blur ------------------------------------------------------------------

/// Approximate a gaussian blur of standard deviation `sigma` (pixels) with
/// three successive box blurs. Works on premultiplied RGBA, which is how
/// `tiny-skia` stores pixels, so edges blur correctly.
fn gaussian_blur(pm: &mut Pixmap, sigma: f32) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let mut tmp = vec![0u8; pm.data().len()];
    for size in box_sizes(sigma) {
        let r = (size - 1) / 2;
        if r == 0 {
            continue;
        }
        box_blur(pm.data(), &mut tmp, w, h, r, true);
        box_blur(&tmp, pm.data_mut(), w, h, r, false);
    }
}

/// Box widths (odd) whose combined blur approximates a gaussian of `sigma`.
fn box_sizes(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n)
        / (-4.0 * wl as f32 - 4.0))
        .round() as i32;
    let pick = |i: i32| if i < m { wl } else { wu } as usize;
    [pick(0), pick(1), pick(2)]
}

/// One box-blur pass of radius `r` along rows (`horizontal`) or columns.
/// Pixels outside the image count as transparent.
fn box_blur(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let (line_step, pix_step) = if horizontal { (w * 4, 4) } else { (4, w * 4) };
    let window = (2 * r + 1) as u32;
    for line in 0..lines {
        let base = line * line_step;
        for c in 0..4 {
            let at = |i: usize| src[base + i * pix_step + c] as u32;
            // Sliding sum over [i - r, i + r].
            let mut sum: u32 = (0..=r.min(len - 1)).map(at).sum();
            for i in 0..len {
                dst[base + i * pix_step + c] = ((sum + window / 2) / window) as u8;
                if i + r + 1 < len {
                    sum += at(i + r + 1);
                }
                if i >= r {
                    sum -= at(i - r);
                }
            }
        }
    }
}

pub fn to_png(pm: &Pixmap) -> Result<Vec<u8>, String> {
    pm.encode_png().map_err(|e| e.to_string())
}

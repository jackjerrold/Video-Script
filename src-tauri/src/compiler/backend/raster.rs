//! Raster backend: draws a frame into a pixel buffer with `tiny-skia`.
//!
//! This is the only place that knows how a shape *looks*. To support a new
//! shape, add a match arm in [`shape_path`].

use crate::compiler::eval::{DrawShape, Frame};
use crate::compiler::schema::ShapeKind;
use tiny_skia::*;

pub fn draw(frame: &Frame) -> Pixmap {
    let mut pm = Pixmap::new(frame.width, frame.height).expect("size is validated by the compiler");
    let bg = frame.background;
    pm.fill(Color::from_rgba8(bg.0, bg.1, bg.2, 255));

    for s in &frame.shapes {
        let opacity = s.num("opacity").clamp(0.0, 1.0);
        if opacity <= 0.0 {
            continue;
        }
        let Some(path) = shape_path(s) else { continue };

        let fill = s.color("fill");
        let mut paint = Paint::default();
        paint.set_color_rgba8(fill.0, fill.1, fill.2, (opacity * 255.0).round() as u8);
        paint.anti_alias = true;

        let scale = s.num("scale") as f32;
        let tf = Transform::from_translate(s.num("x") as f32, s.num("y") as f32)
            .pre_rotate(s.num("rot") as f32)
            .pre_scale(scale, scale);
        pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
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
    }
}

pub fn to_png(pm: &Pixmap) -> Result<Vec<u8>, String> {
    pm.encode_png().map_err(|e| e.to_string())
}

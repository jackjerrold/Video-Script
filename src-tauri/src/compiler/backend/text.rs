//! Text layout: a string becomes one vector path.
//!
//! We turn glyph outlines into a path instead of drawing bitmaps, so text
//! stays sharp under rotation, scaling and group transforms, and animates
//! exactly like any other shape.
//!
//! The font is bundled with `include_bytes!`. To use a different font, drop a
//! `.ttf` into `src-tauri/assets/` and change the path below.

use std::sync::OnceLock;
use tiny_skia::{Path, PathBuilder};
use ttf_parser::{Face, OutlineBuilder};

static FONT_DATA: &[u8] = include_bytes!("../../../assets/Poppins-Medium.ttf");
static FACE: OnceLock<Option<Face<'static>>> = OnceLock::new();

fn face() -> Option<&'static Face<'static>> {
    FACE.get_or_init(|| Face::parse(FONT_DATA, 0).ok()).as_ref()
}

/// Receives one glyph's outline in font units (y up) and appends it to the
/// path in pixels (y down), at the given pen position.
struct GlyphSink<'a> {
    path: &'a mut PathBuilder,
    scale: f32,
    ox: f32,
    oy: f32,
}

impl GlyphSink<'_> {
    fn px(&self, x: f32) -> f32 {
        self.ox + x * self.scale
    }

    fn py(&self, y: f32) -> f32 {
        self.oy - y * self.scale
    }
}

impl OutlineBuilder for GlyphSink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = (self.px(x), self.py(y));
        self.path.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = (self.px(x), self.py(y));
        self.path.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1, x, y) = (self.px(x1), self.py(y1), self.px(x), self.py(y));
        self.path.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1, x2, y2, x, y) = (self.px(x1), self.py(y1), self.px(x2), self.py(y2), self.px(x), self.py(y));
        self.path.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.path.close();
    }
}

fn advance(face: &Face, c: char) -> f32 {
    face.glyph_index(c)
        .and_then(|g| face.glyph_hor_advance(g))
        .map(|a| a as f32)
        .unwrap_or(face.units_per_em() as f32 * 0.5)
}

/// Lay out `text` at `size` pixels tall, centered on the origin both ways.
/// Lines are split on `\n` and each line is centered. `None` if there is
/// nothing to draw (empty, only spaces, or non-positive size).
pub fn text_path(text: &str, size: f32) -> Option<Path> {
    if size <= 0.0 || text.is_empty() {
        return None;
    }
    let face = face()?;
    let scale = size / face.units_per_em() as f32;
    let ascent = face.ascender() as f32 * scale;
    let descent = face.descender() as f32 * scale;
    let line_gap = face.line_gap() as f32 * scale;
    let line_height = ascent - descent + line_gap;

    let lines: Vec<&str> = text.split('\n').collect();
    let total_height = line_height * lines.len() as f32;

    let mut path = PathBuilder::new();
    for (i, line) in lines.iter().enumerate() {
        let width: f32 = line.chars().map(|c| advance(face, c)).sum::<f32>() * scale;
        let mut pen = -width / 2.0;
        let baseline = -total_height / 2.0 + ascent + i as f32 * line_height;
        for c in line.chars() {
            if let Some(glyph) = face.glyph_index(c) {
                let mut sink = GlyphSink { path: &mut path, scale, ox: pen, oy: baseline };
                face.outline_glyph(glyph, &mut sink);
            }
            pen += advance(face, c) * scale;
        }
    }
    path.finish()
}

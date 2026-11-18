//! Text layout: a string becomes one vector path.
//!
//! We turn glyph outlines into a path instead of drawing bitmaps, so text
//! stays sharp under rotation, scaling and group transforms, and animates
//! exactly like any other shape. Layout supports explicit line breaks,
//! word wrapping at a maximum width, left/center/right alignment and a line
//! height multiplier. There is no kerning yet.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    /// Unknown values (the compiler rejects them earlier) fall back to center.
    pub fn parse(s: &str) -> Align {
        match s {
            "left" => Align::Left,
            "right" => Align::Right,
            _ => Align::Center,
        }
    }
}

pub struct TextStyle<'a> {
    pub text: &'a str,
    /// Font size in pixels.
    pub size: f32,
    pub align: Align,
    /// Wrap lines wider than this many pixels. 0 disables wrapping.
    pub max_width: f32,
    /// Multiplier on the font's natural line spacing (0 is treated as 1).
    pub line_height: f32,
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

/// Advance width of one character, in font units.
fn advance(face: &Face, c: char) -> f32 {
    face.glyph_index(c)
        .and_then(|g| face.glyph_hor_advance(g))
        .map(|a| a as f32)
        .unwrap_or(face.units_per_em() as f32 * 0.5)
}

fn width_units(face: &Face, s: &str) -> f32 {
    s.chars().map(|c| advance(face, c)).sum()
}

/// Width in pixels of `text` on a single line at `size`.
pub fn text_width(text: &str, size: f32) -> f32 {
    match face() {
        Some(f) => width_units(f, text) * size / f.units_per_em() as f32,
        None => 0.0,
    }
}

/// Split `text` into display lines.
///
/// Explicit `\n` always starts a new line. With `max_width > 0`, each
/// paragraph is also wrapped greedily at word boundaries so no line is wider
/// than `max_width` pixels; a single word that is too long is broken across
/// lines by character. Wrapping collapses runs of spaces; without wrapping the
/// text is left exactly as written.
pub fn layout_lines(text: &str, size: f32, max_width: f32) -> Vec<String> {
    let paragraphs: Vec<&str> = text.split('\n').collect();
    let (Some(face), true) = (face(), max_width > 0.0 && size > 0.0) else {
        return paragraphs.iter().map(|p| p.to_string()).collect();
    };

    let limit = max_width / (size / face.units_per_em() as f32); // in font units
    let space = advance(face, ' ');
    let mut out = Vec::new();

    for para in paragraphs {
        let words: Vec<&str> = para.split_whitespace().collect();
        if words.is_empty() {
            out.push(String::new()); // keep blank lines
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0.0f32;
        for word in words {
            let ww = width_units(face, word);
            if !cur.is_empty() && cur_w + space + ww <= limit {
                cur.push(' ');
                cur.push_str(word);
                cur_w += space + ww;
                continue;
            }
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                cur_w = 0.0;
            }
            if ww <= limit {
                cur.push_str(word);
                cur_w = ww;
            } else {
                // Longer than a whole line: break it by character.
                for ch in word.chars() {
                    let cw = advance(face, ch);
                    if cur_w + cw > limit && !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                        cur_w = 0.0;
                    }
                    cur.push(ch);
                    cur_w += cw;
                }
            }
        }
        out.push(cur);
    }
    out
}

/// Lay the text out as one path, in pixels, around the origin.
///
/// Horizontally the origin is the anchor selected by `align`: the left edge
/// of the text block for `Left`, its center for `Center`, its right edge for
/// `Right`. The block is `max_width` wide when wrapping, otherwise as wide as
/// its longest line. Vertically the block is always centered on the origin.
/// `None` if there is nothing to draw.
pub fn text_path(st: &TextStyle) -> Option<Path> {
    if st.size <= 0.0 || st.text.is_empty() {
        return None;
    }
    let face = face()?;
    let scale = st.size / face.units_per_em() as f32;
    let ascent = face.ascender() as f32 * scale;
    let descent = face.descender() as f32 * scale;
    let line_gap = face.line_gap() as f32 * scale;
    let natural = ascent - descent + line_gap;
    let pitch = natural * if st.line_height > 0.0 { st.line_height } else { 1.0 };

    let lines = layout_lines(st.text, st.size, st.max_width);
    let widths: Vec<f32> = lines.iter().map(|l| width_units(face, l) * scale).collect();
    let box_w = if st.max_width > 0.0 { st.max_width } else { widths.iter().cloned().fold(0.0, f32::max) };
    let left = match st.align {
        Align::Left => 0.0,
        Align::Center => -box_w / 2.0,
        Align::Right => -box_w,
    };
    let total_height = pitch * lines.len() as f32;

    let mut path = PathBuilder::new();
    for (i, line) in lines.iter().enumerate() {
        let w = widths[i];
        let mut pen = match st.align {
            Align::Left => left,
            Align::Center => left + (box_w - w) / 2.0,
            Align::Right => left + box_w - w,
        };
        let baseline = -total_height / 2.0 + ascent + i as f32 * pitch;
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

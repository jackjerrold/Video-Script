use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use tauri::Emitter;
use tiny_skia::*;

const PROPS: [(&str, f32); 5] = [("x", 0.0), ("y", 0.0), ("rot", 0.0), ("scale", 1.0), ("opacity", 1.0)];

enum Kind { Rect(f32, f32), Circle(f32) }

struct Shape {
    kind: Kind,
    color: [u8; 3],
    keys: Vec<(f32, String, f32)>, // (time, prop, value)
}

struct Scene { w: u32, h: u32, fps: u32, dur: f32, shapes: Vec<Shape> }

impl Shape {
    fn val(&self, prop: &str, def: f32, t: f32) -> f32 {
        let mut k: Vec<(f32, f32)> = self.keys.iter().filter(|k| k.1 == prop).map(|k| (k.0, k.2)).collect();
        if k.is_empty() { return def; }
        k.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        if t <= k[0].0 { return k[0].1; }
        for w in k.windows(2) {
            if t <= w[1].0 {
                let f = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
                return w[0].1 + (w[1].1 - w[0].1) * f;
            }
        }
        k.last().unwrap().1
    }
}

fn num(s: &str) -> Result<f32, String> {
    s.parse::<f32>().map_err(|_| format!("bad number '{s}'"))
}

fn color(s: &str) -> Result<[u8; 3], String> {
    let bad = || format!("bad color '{s}' (use #rrggbb)");
    let h = s.strip_prefix('#').filter(|h| h.len() == 6 && h.is_ascii()).ok_or_else(bad)?;
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| bad());
    Ok([p(0)?, p(2)?, p(4)?])
}

fn parse(src: &str) -> Result<Scene, String> {
    let mut vid = None;
    let mut shapes: Vec<(String, Shape)> = vec![];

    for (i, raw) in src.lines().enumerate() {
        let line = raw.split("//").next().unwrap().trim();
        if line.is_empty() { continue; }
        let t: Vec<&str> = line.split_whitespace().collect();

        (|| -> Result<(), String> {
            match t[0] {
                "video" if t.len() == 5 => {
                    vid = Some((num(t[1])? as u32, num(t[2])? as u32, num(t[3])? as u32, num(t[4])?))
                }
                "rect" if t.len() == 5 => shapes.push((
                    t[1].into(),
                    Shape { kind: Kind::Rect(num(t[2])?, num(t[3])?), color: color(t[4])?, keys: vec![] },
                )),
                "circle" if t.len() == 4 => shapes.push((
                    t[1].into(),
                    Shape { kind: Kind::Circle(num(t[2])?), color: color(t[3])?, keys: vec![] },
                )),
                "at" if t.len() >= 4 => {
                    let time = num(t[1])?;
                    let s = shapes.iter_mut().find(|s| s.0 == t[2]).ok_or(format!("unknown shape '{}'", t[2]))?;
                    for kv in &t[3..] {
                        let (k, v) = kv.split_once('=').ok_or(format!("expected prop=value, got '{kv}'"))?;
                        if !PROPS.iter().any(|p| p.0 == k) { return Err(format!("unknown prop '{k}'")); }
                        s.1.keys.push((time, k.to_string(), num(v)?));
                    }
                }
                _ => return Err(format!("can't parse '{line}'")),
            }
            Ok(())
        })()
        .map_err(|e| format!("line {}: {}", i + 1, e))?;
    }

    let (w, h, fps, dur) = vid.ok_or("missing 'video W H FPS DURATION' line")?;
    if w == 0 || h == 0 || fps == 0 { return Err("video size and fps must be > 0".into()); }
    Ok(Scene { w, h, fps, dur, shapes: shapes.into_iter().map(|s| s.1).collect() })
}

fn render(sc: &Scene, t: f32) -> Pixmap {
    let mut pm = Pixmap::new(sc.w, sc.h).unwrap();
    pm.fill(Color::from_rgba8(20, 20, 28, 255));
    for s in &sc.shapes {
        let (x, y) = (s.val("x", 0.0, t), s.val("y", 0.0, t));
        let (rot, scale) = (s.val("rot", 0.0, t), s.val("scale", 1.0, t));
        let op = s.val("opacity", 1.0, t).clamp(0.0, 1.0);

        let mut paint = Paint::default();
        paint.set_color_rgba8(s.color[0], s.color[1], s.color[2], (op * 255.0) as u8);
        paint.anti_alias = true;

        let path = match s.kind {
            Kind::Rect(w, h) => PathBuilder::from_rect(Rect::from_xywh(-w / 2.0, -h / 2.0, w, h).unwrap()),
            Kind::Circle(r) => PathBuilder::from_circle(0.0, 0.0, r).unwrap(),
        };
        let tf = Transform::from_translate(x, y).pre_rotate(rot).pre_scale(scale, scale);
        pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
    }
    pm
}

#[derive(Serialize)]
struct Preview {
    png: String,
    duration: f32,
    width: u32,
    height: u32,
    fps: u32,
}

#[tauri::command]
fn preview(src: String, t: f32) -> Result<Preview, String> {
    let sc = parse(&src)?;
    let png = render(&sc, t).encode_png().map_err(|e| e.to_string())?;
    Ok(Preview { png: STANDARD.encode(png), duration: sc.dur, width: sc.w, height: sc.h, fps: sc.fps })
}

fn pick_encoder() -> &'static str {
    let text = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    for e in ["libx264", "libopenh264", "mpeg4"] {
        if text.contains(&format!(" {e} ")) {
            return e;
        }
    }
    "mpeg4"
}

#[tauri::command]
async fn export(app: tauri::AppHandle, src: String, path: String) -> Result<String, String> {
    let sc = parse(&src)?;
    let frames = (sc.dur * sc.fps as f32).round() as u32;
    if frames == 0 {
        return Err("duration is too short to produce any frames".into());
    }

    let enc = pick_encoder();
    let size = format!("{}x{}", sc.w, sc.h);
    let fps = sc.fps.to_string();
    let mut args = vec![
        "-y", "-loglevel", "error",
        "-f", "rawvideo", "-pix_fmt", "rgba",
        "-s", size.as_str(), "-r", fps.as_str(),
        "-i", "-",
        "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2",
        "-c:v", enc, "-pix_fmt", "yuv420p",
    ];
    if enc != "libx264" {
        args.extend(["-b:v", "8M"]); // the fallback encoders need an explicit bitrate
    }
    args.extend(["-movflags", "+faststart", path.as_str()]);

    let mut ff = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start ffmpeg (is it installed and on PATH?): {e}"))?;

    // Drain stderr on a thread so ffmpeg can never block on a full pipe.
    let mut stderr = ff.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let mut stdin = ff.stdin.take().unwrap();
    let mut write_failed = false;
    for i in 0..frames {
        let pm = render(&sc, i as f32 / sc.fps as f32);
        if stdin.write_all(pm.data()).is_err() {
            write_failed = true; // ffmpeg died; its stderr will tell us why
            break;
        }
        let _ = app.emit("export-progress", (i + 1) as f32 / frames as f32);
    }
    drop(stdin);

    let status = ff.wait().map_err(|e| e.to_string())?;
    let log = reader.join().unwrap_or_default();

    if status.success() && !write_failed {
        Ok(format!("Saved {path}"))
    } else {
        let lines: Vec<&str> = log.lines().rev().take(3).collect();
        let detail = lines.into_iter().rev().collect::<Vec<_>>().join(" | ");
        Err(if detail.is_empty() { "ffmpeg failed with no output".into() } else { format!("ffmpeg: {detail}") })
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![preview, export])
        .run(tauri::generate_context!())
        .expect("error while running tauri app");
}
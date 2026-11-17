//! Video export: render every frame and pipe it to `ffmpeg`.

use crate::compiler::backend::raster;
use crate::compiler::eval;
use crate::compiler::ir::Program;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// Pick the best H.264-ish encoder this ffmpeg build has.
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

/// Render the whole video to `path`. `on_progress` receives 0.0..=1.0.
///
/// Frames are sampled so the first is at t=0 and the last at t=duration,
/// meaning a keyframe placed exactly at the end of the video is reached.
pub fn export_mp4(program: &Program, path: &str, mut on_progress: impl FnMut(f32)) -> Result<String, String> {
    let video = &program.video;
    let frames = video.frame_count();
    let enc = pick_encoder();
    let size = format!("{}x{}", video.width, video.height);
    let fps = video.fps.to_string();

    let mut args = vec![
        "-y", "-loglevel", "error",
        "-f", "rawvideo", "-pix_fmt", "rgba",
        "-s", size.as_str(), "-r", fps.as_str(),
        "-i", "-",
        "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2", // H.264 needs even dimensions
        "-c:v", enc, "-pix_fmt", "yuv420p",
    ];
    if enc != "libx264" {
        args.extend(["-b:v", "8M"]); // the fallback encoders need an explicit bitrate
    }
    args.extend(["-movflags", "+faststart", path]);

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
        let t = if frames > 1 { video.duration * i as f64 / (frames - 1) as f64 } else { 0.0 };
        let pm = raster::draw(&eval::eval(program, t));
        if stdin.write_all(pm.data()).is_err() {
            write_failed = true; // ffmpeg died; its stderr says why
            break;
        }
        on_progress((i + 1) as f32 / frames as f32);
    }
    drop(stdin);

    let status = ff.wait().map_err(|e| e.to_string())?;
    let log = reader.join().unwrap_or_default();
    if status.success() && !write_failed {
        Ok(format!("Saved {path} ({frames} frames, {enc})"))
    } else {
        let lines: Vec<&str> = log.lines().rev().take(3).collect();
        let detail = lines.into_iter().rev().collect::<Vec<_>>().join(" | ");
        Err(if detail.is_empty() { "ffmpeg failed with no output".into() } else { format!("ffmpeg: {detail}") })
    }
}

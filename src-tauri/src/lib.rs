//! Tauri glue. All language logic lives in `compiler/`; ffmpeg export in `video.rs`.
//! This file only exposes the compiler to the frontend as commands.

mod compiler;
mod video;

use base64::{engine::general_purpose::STANDARD, Engine};
use compiler::diagnostics::{DiagnosticView, Severity};
use compiler::ir::Program;
use compiler::span::Source;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

/// The last compile result. Preview is called on every slider tick, but the
/// script only changes when the user types, so we compile once per edit.
struct Cached {
    src: String,
    program: Option<Arc<Program>>,
    diagnostics: Vec<DiagnosticView>,
}

#[derive(Default)]
struct Cache(Mutex<Option<Cached>>);

fn compile_cached(cache: &Cache, src: &str) -> (Option<Arc<Program>>, Vec<DiagnosticView>) {
    let mut guard = cache.0.lock().unwrap();
    if let Some(c) = guard.as_ref() {
        if c.src == src {
            return (c.program.clone(), c.diagnostics.clone());
        }
    }
    let source = Source::new(src);
    let out = compiler::compile(&source);
    let views: Vec<DiagnosticView> = out.diagnostics.iter().map(|d| d.view(&source)).collect();
    let program = out.program.map(Arc::new);
    *guard = Some(Cached { src: src.to_string(), program: program.clone(), diagnostics: views.clone() });
    (program, views)
}

#[derive(Serialize)]
struct VideoInfo {
    width: u32,
    height: u32,
    fps: u32,
    duration: f64,
}

#[derive(Serialize)]
struct Preview {
    /// `None` when the script has errors; the UI keeps showing the last good frame.
    png: Option<String>,
    info: Option<VideoInfo>,
    diagnostics: Vec<DiagnosticView>,
}

#[tauri::command]
fn preview(cache: tauri::State<'_, Cache>, src: String, t: f64) -> Preview {
    let (program, diagnostics) = compile_cached(cache.inner(), &src);
    let (png, info) = match program {
        Some(p) => {
            let frame = compiler::eval::eval(&p, t);
            let png = compiler::backend::raster::to_png(&compiler::backend::raster::draw(&frame))
                .ok()
                .map(|bytes| STANDARD.encode(bytes));
            let v = &p.video;
            (png, Some(VideoInfo { width: v.width, height: v.height, fps: v.fps, duration: v.duration }))
        }
        None => (None, None),
    };
    Preview { png, info, diagnostics }
}

/// Dump an intermediate stage ("tokens", "ast" or "ir") as text, for the debug tabs.
#[tauri::command]
fn debug_stage(src: String, stage: String) -> Result<String, String> {
    compiler::debug_stage(&Source::new(src), &stage)
}

#[tauri::command]
async fn export(
    app: tauri::AppHandle,
    cache: tauri::State<'_, Cache>,
    src: String,
    path: String,
) -> Result<String, String> {
    let (program, diagnostics) = compile_cached(cache.inner(), &src);
    let Some(program) = program else {
        let first = diagnostics.iter().find(|d| d.severity == Severity::Error);
        return Err(match first {
            Some(d) => format!("fix the errors first (line {}: {})", d.line, d.message),
            None => "nothing to export".into(),
        });
    };

    // Rendering is CPU-heavy: keep it off the async runtime threads.
    let result = tauri::async_runtime::spawn_blocking(move || {
        video::export_mp4(&program, &path, |p| {
            let _ = app.emit("export-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Cache::default())
        .invoke_handler(tauri::generate_handler![preview, debug_stage, export])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

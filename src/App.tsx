import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Preview = { png: string; duration: number; width: number; height: number; fps: number };

const DEFAULT = `video 640 360 30 4
rect box 80 80 #ff5555
circle ball 30 #55aaff

at 0 box x=60 y=180 rot=0
at 4 box x=580 y=180 rot=360

at 0 ball x=320 y=40 scale=1
at 2 ball x=320 y=320 scale=2
at 4 ball x=320 y=40 scale=1
`;

export default function App() {
  const [src, setSrc] = useState(DEFAULT);
  const [t, setT] = useState(0);
  const [info, setInfo] = useState<Preview | null>(null);
  const [error, setError] = useState("");
  const [playing, setPlaying] = useState(false);
  const [out, setOut] = useState("/tmp/out.mp4");
  const [exporting, setExporting] = useState(false);
  const [progress, setProgress] = useState(0);
  const [exportMsg, setExportMsg] = useState("");

  const sent = useRef(0);
  const shown = useRef(0);
  const dur = info?.duration ?? 4;

  // Render a preview frame whenever the script or time changes.
  useEffect(() => {
    const id = ++sent.current;
    invoke<Preview>("preview", { src, t })
      .then((r) => {
        if (id < shown.current) return; // ignore out-of-order responses
        shown.current = id;
        setInfo(r);
        setError("");
        if (t > r.duration) setT(r.duration);
      })
      .catch((e) => {
        if (id < shown.current) return;
        shown.current = id;
        setError(String(e));
      });
  }, [src, t]);

  // Playback loop.
  useEffect(() => {
    if (!playing) return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = (now - last) / 1000;
      last = now;
      setT((p) => (p + dt >= dur ? 0 : p + dt));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing, dur]);

  // Export progress events from Rust.
  useEffect(() => {
    const un = listen<number>("export-progress", (e) => setProgress(e.payload));
    return () => { un.then((f) => f()); };
  }, []);

  const doExport = async () => {
    setPlaying(false);
    setExporting(true);
    setProgress(0);
    setExportMsg("");
    try {
      setExportMsg(await invoke<string>("export", { src, path: out }));
    } catch (e) {
      setExportMsg("Export failed: " + e);
    } finally {
      setExporting(false);
    }
  };

  const failed = exportMsg.startsWith("Export failed");

  return (
    <div className="app">
      <header>
        <div className="logo">▶ Keyframe Studio</div>
        <div className="spacer" />
        <input
          className="path"
          value={out}
          onChange={(e) => setOut(e.target.value)}
          placeholder="/path/to/output.mp4"
          spellCheck={false}
        />
        <button className="primary" onClick={doExport} disabled={exporting || !!error}>
          {exporting ? "Exporting…" : "Export MP4"}
        </button>
      </header>

      <main>
        <section className="panel editor">
          <div className="panel-title">Script</div>
          <textarea value={src} onChange={(e) => setSrc(e.target.value)} spellCheck={false} />
        </section>

        <section className="panel viewer">
          <div className="panel-title">
            Preview
            {info && <span className="meta">{info.width}×{info.height} · {info.fps} fps · {info.duration}s</span>}
          </div>
          <div className="stage">
            {info && <img src={`data:image/png;base64,${info.png}`} alt="preview" />}
          </div>
          <div className="controls">
            <button onClick={() => setPlaying((p) => !p)}>{playing ? "⏸" : "▶"}</button>
            <input
              type="range"
              min={0}
              max={dur}
              step={0.01}
              value={Math.min(t, dur)}
              onChange={(e) => { setPlaying(false); setT(+e.target.value); }}
            />
            <span className="time">{t.toFixed(2)} / {dur.toFixed(2)}s</span>
          </div>
        </section>
      </main>

      <footer className={error || failed ? "status error" : "status"}>
        {error ? (
          <>⚠ {error}</>
        ) : exporting ? (
          <>
            <span>Exporting {Math.round(progress * 100)}%</span>
            <div className="bar"><div style={{ width: `${progress * 100}%` }} /></div>
          </>
        ) : (
          exportMsg || "Ready"
        )}
      </footer>
    </div>
  );
}
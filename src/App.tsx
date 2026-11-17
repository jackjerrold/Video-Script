import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Diag = {
  severity: "error" | "warning";
  code: string;
  message: string;
  line: number;
  col: number;
  end_line: number;
  end_col: number;
  help: string | null;
  rendered: string;
};
type Info = { width: number; height: number; fps: number; duration: number };
type PreviewResult = { png: string | null; info: Info | null; diagnostics: Diag[] };
type Tab = "preview" | "tokens" | "ast" | "ir";

const TABS: { id: Tab; label: string }[] = [
  { id: "preview", label: "Preview" },
  { id: "tokens", label: "Tokens" },
  { id: "ast", label: "AST" },
  { id: "ir", label: "IR" },
];

const DEFAULT = `// Keyframe Studio: try editing me!
video {
  width: 640;
  height: 360;
  fps: 30;
  duration: 4s;
  background: #14141c;
}

let red  = #ff5555;
let blue = #55aaff;
let mid  = height / 2;

rect box {
  x: 60;
  y: mid;
  w: 80;
  h: 80;
  fill: red;
}

circle ball {
  x: width / 2;
  y: 40;
  r: 30;
  fill: blue;
}

// A keyframe sets values at a time. 'ease' shapes the move that ends there.
at 4s box ease in_out {
  x: width - 60;
  rot: 360;
  fill: #ffcc00;
}

at 2s ball ease in_out { y: height - 40; r: 60; }
at 4s ball ease in_out { y: 40; r: 30; }
`;

export default function App() {
  const [src, setSrc] = useState(DEFAULT);
  const [t, setT] = useState(0);
  const [img, setImg] = useState("");
  const [info, setInfo] = useState<Info | null>(null);
  const [diags, setDiags] = useState<Diag[]>([]);
  const [tab, setTab] = useState<Tab>("preview");
  const [debug, setDebug] = useState("");
  const [playing, setPlaying] = useState(false);
  const [out, setOut] = useState("/tmp/out.mp4");
  const [exporting, setExporting] = useState(false);
  const [progress, setProgress] = useState(0);
  const [exportMsg, setExportMsg] = useState("");

  const editor = useRef<HTMLTextAreaElement>(null);
  const sent = useRef(0);
  const shown = useRef(0);

  const dur = info?.duration ?? 4;
  const errors = diags.filter((d) => d.severity === "error").length;
  const warnings = diags.length - errors;

  // Compile + render a preview frame whenever the script or time changes.
  useEffect(() => {
    const id = ++sent.current;
    invoke<PreviewResult>("preview", { src, t })
      .then((r) => {
        if (id < shown.current) return; // ignore out-of-order responses
        shown.current = id;
        setDiags(r.diagnostics);
        if (r.png) setImg(r.png); // on errors, keep the last good frame
        if (r.info) {
          setInfo(r.info);
          if (t > r.info.duration) setT(r.info.duration);
        }
      })
      .catch((e) => setExportMsg(String(e)));
  }, [src, t]);

  // Debug tabs show an intermediate compiler stage.
  useEffect(() => {
    if (tab === "preview") return;
    invoke<string>("debug_stage", { src, stage: tab })
      .then(setDebug)
      .catch((e) => setDebug(String(e)));
  }, [src, tab]);

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
    return () => {
      un.then((f) => f());
    };
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

  // Click a diagnostic to select the offending text in the editor.
  const jumpTo = (d: Diag) => {
    const el = editor.current;
    if (!el) return;
    const lines = src.split("\n");
    let start = 0;
    for (let i = 0; i < d.line - 1; i++) start += lines[i].length + 1;
    start += d.col - 1;
    const len = d.end_line === d.line ? Math.max(1, d.end_col - d.col) : 1;
    el.focus();
    el.setSelectionRange(start, start + len);
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
        <button className="primary" onClick={doExport} disabled={exporting || errors > 0}>
          {exporting ? "Exporting…" : "Export MP4"}
        </button>
      </header>

      <main>
        <section className="panel editor">
          <div className="panel-title">Script</div>
          <textarea ref={editor} value={src} onChange={(e) => setSrc(e.target.value)} spellCheck={false} />
          <div className="diags">
            {diags.length === 0 ? (
              <div className="diag-ok">✓ No problems</div>
            ) : (
              diags.map((d, i) => (
                <div key={i} className={`diag ${d.severity}`} onClick={() => jumpTo(d)}>
                  <div>
                    <span className="code">{d.code}</span>
                    <span className="loc">
                      {d.line}:{d.col}
                    </span>
                    {d.message}
                  </div>
                  {d.help && <div className="help">{d.help}</div>}
                </div>
              ))
            )}
          </div>
        </section>

        <section className="panel viewer">
          <div className="panel-title">
            <div className="tabs">
              {TABS.map((x) => (
                <button key={x.id} className={tab === x.id ? "tab active" : "tab"} onClick={() => setTab(x.id)}>
                  {x.label}
                </button>
              ))}
            </div>
            {info && (
              <span className="meta">
                {info.width}×{info.height} · {info.fps} fps · {info.duration}s
              </span>
            )}
          </div>

          {tab === "preview" ? (
            <>
              <div className="stage">{img && <img src={`data:image/png;base64,${img}`} alt="preview" />}</div>
              <div className="controls">
                <button onClick={() => setPlaying((p) => !p)}>{playing ? "⏸" : "▶"}</button>
                <input
                  type="range"
                  min={0}
                  max={dur}
                  step={0.01}
                  value={Math.min(t, dur)}
                  onChange={(e) => {
                    setPlaying(false);
                    setT(+e.target.value);
                  }}
                />
                <span className="time">
                  {t.toFixed(2)} / {dur.toFixed(2)}s
                </span>
              </div>
            </>
          ) : (
            <pre className="debug">{debug}</pre>
          )}
        </section>
      </main>

      <footer className={errors > 0 || failed ? "status error" : "status"}>
        {exporting ? (
          <>
            <span>Exporting {Math.round(progress * 100)}%</span>
            <div className="bar">
              <div style={{ width: `${progress * 100}%` }} />
            </div>
          </>
        ) : exportMsg ? (
          exportMsg
        ) : errors > 0 ? (
          `${errors} error${errors > 1 ? "s" : ""}${warnings ? `, ${warnings} warning${warnings > 1 ? "s" : ""}` : ""}`
        ) : warnings > 0 ? (
          `${warnings} warning${warnings > 1 ? "s" : ""}`
        ) : (
          "Ready"
        )}
      </footer>
    </div>
  );
}
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EXAMPLES } from "./examples";
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
type Param = {
  name: string;
  kind: "slider" | "color" | "text" | "choice";
  value: number | string;
  default: number | string;
  min: number | null;
  max: number | null;
  step: number | null;
  choices: string[];
};
type Overrides = Record<string, number | string>;
type PreviewResult = { png: string | null; info: Info | null; params: Param[] | null; diagnostics: Diag[] };
type Tab = "preview" | "reference" | "tokens" | "ast" | "ir";

const TABS: { id: Tab; label: string }[] = [
  { id: "preview", label: "Preview" },
  { id: "reference", label: "Reference" },
  { id: "tokens", label: "Tokens" },
  { id: "ast", label: "AST" },
  { id: "ir", label: "IR" },
];

function ParamRow({ p, onChange }: { p: Param; onChange: (v: number | string) => void }) {
  const changed = p.value !== p.default;
  let control;
  if (p.kind === "slider") {
    const v = p.value as number;
    const lo = Math.min(p.min ?? 0, v);
    const hi = Math.max(p.max ?? 100, v);
    const step = p.step ?? (hi - lo > 20 ? 1 : 0.01);
    control = (
      <>
        <input type="range" min={lo} max={hi} step={step} value={v} onChange={(e) => onChange(+e.target.value)} />
        <input
          className="num"
          type="number"
          step={step}
          value={Number.isFinite(v) ? +v.toFixed(4) : 0}
          onChange={(e) => e.target.value !== "" && onChange(+e.target.value)}
        />
      </>
    );
  } else if (p.kind === "color") {
    const s = p.value as string;
    control = (
      <>
        <input
          type="color"
          value={s.slice(0, 7)}
          onChange={(e) => onChange(e.target.value + (s.length === 9 ? s.slice(7) : ""))}
        />
        <code>{s}</code>
      </>
    );
  } else if (p.kind === "choice") {
    control = (
      <select value={p.value as string} onChange={(e) => onChange(e.target.value)}>
        {p.choices.map((c) => (
          <option key={c} value={c}>
            {c}
          </option>
        ))}
      </select>
    );
  } else {
    control = <input className="text" value={p.value as string} onChange={(e) => onChange(e.target.value)} />;
  }
  return (
    <div className={changed ? "param changed" : "param"}>
      <label title={p.name}>{p.name.replace(/_/g, " ")}</label>
      {control}
      <button className="reset" disabled={!changed} onClick={() => onChange(p.default)} title="Back to the script's value">
        ↺
      </button>
    </div>
  );
}

export default function App() {
  const [src, setSrc] = useState(() => {
    try {
      return localStorage.getItem("kfs.script") ?? EXAMPLES[0].src;
    } catch {
      return EXAMPLES[0].src;
    }
  });
  const [t, setT] = useState(0);
  const [img, setImg] = useState("");
  const [info, setInfo] = useState<Info | null>(null);
  const [diags, setDiags] = useState<Diag[]>([]);
  const [tab, setTab] = useState<Tab>("preview");
  const [debug, setDebug] = useState("");
  const [params, setParams] = useState<Param[]>([]);
  const [overrides, setOverrides] = useState<Overrides>({});
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
    invoke<PreviewResult>("preview", { src, t, overrides })
      .then((r) => {
        if (id < shown.current) return; // ignore out-of-order responses
        shown.current = id;
        setDiags(r.diagnostics);
        if (r.params) setParams(r.params); // on errors, keep the last good controls
        if (r.png) setImg(r.png); // on errors, keep the last good frame
        if (r.info) {
          setInfo(r.info);
          if (t > r.info.duration) setT(r.info.duration);
        }
      })
      .catch((e) => setExportMsg(String(e)));
  }, [src, t, overrides]);

  // Autosave the script.
  useEffect(() => {
    try {
      localStorage.setItem("kfs.script", src);
    } catch {
      /* storage unavailable: ignore */
    }
  }, [src]);

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

  const setParam = (p: Param, v: number | string) => {
    setOverrides((o) => {
      const next = { ...o };
      if (v === p.default) delete next[p.name];
      else next[p.name] = v;
      return next;
    });
  };

  const bake = async () => {
    try {
      setSrc(await invoke<string>("bake", { src, overrides }));
      setOverrides({});
    } catch (e) {
      setExportMsg(String(e));
    }
  };

  const modified = params.filter((p) => p.value !== p.default).length;

  const doExport = async () => {
    setPlaying(false);
    setExporting(true);
    setProgress(0);
    setExportMsg("");
    try {
      setExportMsg(await invoke<string>("export", { src, path: out, overrides }));
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
        <select
          className="path"
          style={{ flex: "0 0 auto", width: "auto" }}
          value=""
          onChange={(e) => {
            const ex = EXAMPLES.find((x) => x.id === e.target.value);
            if (ex) {
              setSrc(ex.src);
              setT(0);
              setPlaying(false);
            }
          }}
        >
          <option value="" disabled>
            Examples…
          </option>
          {EXAMPLES.map((x) => (
            <option key={x.id} value={x.id}>
              {x.label}
            </option>
          ))}
        </select>
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
              {params.length > 0 && (
                <div className="inspector">
                  <div className="inspector-head">
                    <span>Inspector</span>
                    <div className="spacer" />
                    <button disabled={modified === 0} onClick={() => setOverrides({})}>
                      Reset
                    </button>
                    <button disabled={modified === 0} onClick={bake} title="Write these values into the script">
                      Bake into script
                    </button>
                  </div>
                  <div className="params">
                    {params.map((p) => (
                      <ParamRow key={p.name} p={p} onChange={(v) => setParam(p, v)} />
                    ))}
                  </div>
                </div>
              )}
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

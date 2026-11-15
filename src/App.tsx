import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

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
  const [dur, setDur] = useState(4);
  const [img, setImg] = useState("");
  const [msg, setMsg] = useState("");
  const [out, setOut] = useState("/tmp/out.mp4");

  useEffect(() => {
    invoke<{ png: string; duration: number }>("preview", { src, t })
      .then((r) => { setImg(r.png); setDur(r.duration); setMsg(""); })
      .catch((e) => setMsg(String(e)));
  }, [src, t]);

  const doExport = () => {
    setMsg("exporting...");
    invoke<string>("export", { src, path: out })
      .then(setMsg)
      .catch((e) => setMsg(String(e)));
  };

  return (
    <div style={{ display: "flex", gap: 16, padding: 16, fontFamily: "sans-serif" }}>
      <textarea
        value={src}
        onChange={(e) => setSrc(e.target.value)}
        spellCheck={false}
        style={{ width: 420, height: 400, fontFamily: "monospace" }}
      />
      <div>
        {img && <img src={`data:image/png;base64,${img}`} style={{ maxWidth: 640 }} />}
        <input type="range" min={0} max={dur} step={0.01} value={t}
               onChange={(e) => setT(+e.target.value)} style={{ width: "100%" }} />
        <div>t = {t.toFixed(2)}s</div>
        <input value={out} onChange={(e) => setOut(e.target.value)} style={{ width: 300 }} />
        <button onClick={doExport}>Export MP4</button>
        <pre style={{ color: msg.startsWith("line") ? "crimson" : "inherit" }}>{msg}</pre>
      </div>
    </div>
  );
}
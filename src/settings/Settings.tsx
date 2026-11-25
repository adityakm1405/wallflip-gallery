import { useEffect, useRef, useState } from "react";
import {
  chooseFolder,
  getAutostart,
  getConfig,
  previewTransition,
  quitApp,
  saveConfig,
  setAutostart,
} from "../lib/api";
import type { Config, CycleMode, Fit, StyleSetting, TransitionName } from "../lib/api";
import { recordHotkey } from "./hotkey";

const STYLES: { id: StyleSetting; label: string; hint: string }[] = [
  { id: "carousel", label: "Carousel", hint: "Flat strip, centre card in focus" },
  { id: "depth", label: "Depth", hint: "Cards recede behind the active one" },
  { id: "fan", label: "Fan", hint: "A hand of cards fanned in an arc" },
  { id: "wall", label: "Wall", hint: "Scrolling grid of every image" },
  { id: "random", label: "Random", hint: "A different style each time" },
];

const TRANSITIONS: { id: TransitionName; label: string }[] = [
  { id: "circle", label: "Circle reveal" },
  { id: "wipe", label: "Wipe" },
  { id: "diagonal", label: "Diagonal" },
  { id: "diamond", label: "Diamond" },
  { id: "fade", label: "Fade" },
  { id: "slide", label: "Slide" },
  { id: "push", label: "Push" },
  { id: "zoom", label: "Zoom in" },
  { id: "zoomout", label: "Burst" },
  { id: "blur", label: "Blur" },
  { id: "blinds", label: "Blinds" },
  { id: "tiles", label: "Tiles" },
  { id: "flip", label: "Flip" },
  { id: "random", label: "Random" },
  { id: "native", label: "Windows fade" },
];

const MODES: { id: CycleMode; label: string }[] = [
  { id: "norepeat", label: "No-repeat" },
  { id: "shuffle", label: "Shuffle" },
  { id: "sequential", label: "Sequential" },
];

const FITS: { id: Fit; label: string }[] = [
  { id: "fill", label: "Fill" },
  { id: "fit", label: "Fit" },
  { id: "stretch", label: "Stretch" },
  { id: "tile", label: "Tile" },
  { id: "center", label: "Center" },
  { id: "span", label: "Span (all monitors)" },
];

const INTERVALS = [0, 1, 5, 15, 30, 60, 180, 720];
const intervalLabel = (m: number) =>
  m === 0 ? "Off" : m < 60 ? `Every ${m} min` : `Every ${m / 60} h`;

function HotkeyField({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (v: string) => void;
  label: string;
}) {
  const [recording, setRecording] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const ref = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
        setNote(null);
        return;
      }
      const r = recordHotkey(e);
      if (r.kind === "ok") {
        onChange(r.combo);
        setRecording(false);
        setNote(null);
      } else if (r.kind === "invalid") {
        setNote(r.reason);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, onChange]);

  return (
    <div className="hotkey">
      <button
        ref={ref}
        type="button"
        className={"keybox" + (recording ? " recording" : "")}
        aria-label={label}
        onClick={() => {
          setRecording(true);
          setNote(null);
        }}
        onBlur={() => setRecording(false)}
      >
        {recording ? "Press a key combination…" : value.split("+").map((k, i) => <kbd key={i}>{k}</kbd>)}
      </button>
      {note && <span className="note warn">{note}</span>}
    </div>
  );
}

export function Settings() {
  const [cfg, setCfg] = useState<Config | null>(null);
  const [saved, setSaved] = useState<Config | null>(null);
  const [autostart, setAuto] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void getConfig().then((c) => {
      setCfg(c);
      setSaved(c);
    });
    void getAutostart().then(setAuto).catch(() => undefined);
  }, []);

  if (!cfg) return <div className="loading">Loading…</div>;

  const set = <K extends keyof Config>(key: K, value: Config[K]) => setCfg({ ...cfg, [key]: value });
  const dirty = JSON.stringify(cfg) !== JSON.stringify(saved);

  const browse = async () => {
    const f = await chooseFolder();
    if (f) set("folder", f);
  };

  const save = async () => {
    setBusy(true);
    setStatus(null);
    try {
      await saveConfig(cfg);
      setSaved(cfg);
      setStatus({ ok: true, text: "Saved" });
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const tryIt = async () => {
    setStatus(null);
    try {
      await previewTransition(cfg.transition, cfg.transition_ms);
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    }
  };

  const toggleAutostart = async (on: boolean) => {
    setAuto(on);
    try {
      await setAutostart(on);
    } catch (e) {
      setAuto(!on);
      setStatus({ ok: false, text: String(e) });
    }
  };

  return (
    <main>
      <header>
        <h1>WallFlip</h1>
        <p>Press your gallery shortcut anywhere to browse wallpapers.</p>
      </header>

      <section>
        <h2>Wallpaper folder</h2>
        <div className="row">
          <div className="path" title={cfg.folder ?? ""}>
            {cfg.folder ?? "No folder chosen"}
          </div>
          <button className="btn" onClick={() => void browse()}>
            Browse…
          </button>
        </div>
        <label className="check">
          <input
            type="checkbox"
            checked={cfg.recursive}
            onChange={(e) => set("recursive", e.target.checked)}
          />
          Include images from subfolders
        </label>
      </section>

      <section>
        <h2>Gallery style</h2>
        <div className="chips">
          {STYLES.map((s) => (
            <button
              key={s.id}
              className={"chip" + (cfg.style === s.id ? " on" : "")}
              onClick={() => set("style", s.id)}
              title={s.hint}
            >
              {s.label}
            </button>
          ))}
        </div>
        <p className="hint">{STYLES.find((s) => s.id === cfg.style)?.hint}. Press Tab in the gallery to try the others.</p>
      </section>

      <section>
        <h2>Change animation</h2>
        <div className="chips">
          {TRANSITIONS.map((t) => (
            <button
              key={t.id}
              className={"chip" + (cfg.transition === t.id ? " on" : "")}
              onClick={() => set("transition", t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="grid2">
          <span>Duration · {(cfg.transition_ms / 1000).toFixed(1)} s</span>
          <input
            type="range"
            min={200}
            max={3000}
            step={100}
            value={cfg.transition_ms}
            disabled={cfg.transition === "native"}
            onChange={(e) => set("transition_ms", Number(e.target.value))}
          />
        </div>
        <div className="row" style={{ marginTop: 10 }}>
          <button className="btn" onClick={() => void tryIt()} disabled={!cfg.folder}>
            Try it (next wallpaper)
          </button>
        </div>
        <p className="hint">
          Plays when you pick from the gallery or use the quick-cycle shortcut; the screen shows the desktop for
          the length of the animation. Timed auto-change always uses the Windows fade so it never covers your windows.
        </p>
      </section>

      <section>
        <h2>Shortcuts</h2>
        <div className="grid2">
          <span>Open gallery</span>
          <HotkeyField
            label="Gallery shortcut"
            value={cfg.gallery_hotkey}
            onChange={(v) => set("gallery_hotkey", v)}
          />
          <span>Instant next wallpaper</span>
          <HotkeyField
            label="Quick-cycle shortcut"
            value={cfg.cycle_hotkey}
            onChange={(v) => set("cycle_hotkey", v)}
          />
        </div>
      </section>

      <section>
        <h2>Quick cycle &amp; automation</h2>
        <div className="chips">
          {MODES.map((m) => (
            <button
              key={m.id}
              className={"chip" + (cfg.mode === m.id ? " on" : "")}
              onClick={() => set("mode", m.id)}
            >
              {m.label}
            </button>
          ))}
        </div>
        <div className="grid2">
          <span>Picture position</span>
          <select value={cfg.fit} onChange={(e) => set("fit", e.target.value as Fit)}>
            {FITS.map((f) => (
              <option key={f.id} value={f.id}>
                {f.label}
              </option>
            ))}
          </select>
          <span>Auto-change</span>
          <select
            value={cfg.interval_min}
            onChange={(e) => set("interval_min", Number(e.target.value))}
          >
            {(INTERVALS.includes(cfg.interval_min) ? INTERVALS : [...INTERVALS, cfg.interval_min]).map(
              (m) => (
                <option key={m} value={m}>
                  {intervalLabel(m)}
                </option>
              ),
            )}
          </select>
        </div>
      </section>

      <section>
        <label className="check">
          <input type="checkbox" checked={autostart} onChange={(e) => void toggleAutostart(e.target.checked)} />
          Start WallFlip with Windows
        </label>
      </section>

      <footer>
        <button className="btn ghost danger" onClick={() => void quitApp()}>
          Quit WallFlip
        </button>
        <span className={"status" + (status ? (status.ok ? " ok" : " err") : "")}>{status?.text}</span>
        <button className="btn primary" disabled={!dirty || busy} onClick={() => void save()}>
          {busy ? "Saving…" : "Save"}
        </button>
      </footer>
    </main>
  );
}

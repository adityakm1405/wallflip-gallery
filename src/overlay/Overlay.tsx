import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  applyWallpaper,
  getGallery,
  hideOverlay,
  openSettings,
  reloadGallery,
} from "../lib/api";
import type { Gallery, StyleName } from "../lib/api";
import { Card } from "./Card";
import { Wall } from "./Wall";
import { cardLook, REACH, sizeFor } from "./layouts";

const STYLES: StyleName[] = ["carousel", "depth", "fan", "wall"];
const STYLE_LABEL: Record<StyleName, string> = {
  carousel: "Carousel",
  depth: "Depth",
  fan: "Fan",
  wall: "Wall",
};

function resolveStyle(setting: Gallery["style"]): StyleName {
  if (setting === "random") return STYLES[Math.floor(Math.random() * STYLES.length)];
  return setting;
}

function prettyName(file: string): string {
  return file.replace(/\.[^.]+$/, "").replace(/[_]+/g, " ");
}

export function Overlay() {
  const [gallery, setGallery] = useState<Gallery | null>(null);
  const [active, setActive] = useState(0);
  const [style, setStyle] = useState<StyleName>("carousel");
  const [shown, setShown] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [vp, setVp] = useState({ w: window.innerWidth, h: window.innerHeight });
  const cols = useRef(1);
  const toastTimer = useRef<number>(0);
  const wheelAt = useRef(0);

  const items = gallery?.items ?? [];
  const n = items.length;

  const say = useCallback((msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 1800);
  }, []);

  /** Load the folder listing and (re)focus the wallpaper that is on screen now. */
  const load = useCallback(
    async (fresh: boolean) => {
      try {
        const g = await (fresh ? reloadGallery() : getGallery());
        setGallery(g);
        const cur = g.current ? g.items.findIndex((i) => i.id === g.current) : -1;
        setActive((prev) => (fresh ? Math.min(prev, Math.max(0, g.items.length - 1)) : Math.max(0, cur)));
        return g;
      } catch (e) {
        say(String(e));
        return null;
      }
    },
    [say],
  );

  // First load + show/hide events from the Rust side.
  useEffect(() => {
    void load(false).then((g) => g && setStyle(resolveStyle(g.style)));
    const unlisten = [
      listen("overlay-shown", async () => {
        setShown(false);
        const g = await load(false);
        if (g) setStyle(resolveStyle(g.style));
        // two frames: lets the "hidden" state paint once so the entrance animates
        requestAnimationFrame(() => requestAnimationFrame(() => setShown(true)));
      }),
      listen("overlay-hidden", () => setShown(false)),
      listen("config-changed", () => void load(true)),
    ];
    return () => {
      unlisten.forEach((p) => void p.then((f) => f()));
    };
  }, [load]);

  useEffect(() => {
    const onResize = () => setVp({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  const move = useCallback(
    (delta: number) => {
      if (n === 0) return;
      setActive((a) => Math.min(n - 1, Math.max(0, a + delta)));
    },
    [n],
  );

  const apply = useCallback(
    async (index: number) => {
      const item = items[index];
      if (!item) return;
      try {
        await applyWallpaper(item.id); // Rust hides the overlay on success
      } catch (e) {
        say(String(e));
      }
    },
    [items, say],
  );

  const cycleStyle = useCallback(() => {
    setStyle((s) => {
      const next = STYLES[(STYLES.indexOf(s) + 1) % STYLES.length];
      say(`Style: ${STYLE_LABEL[next]}`);
      return next;
    });
  }, [say]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.altKey || e.metaKey) return;
      const wall = style === "wall";
      switch (e.key) {
        case "ArrowRight":
        case "l":
          move(1);
          break;
        case "ArrowLeft":
        case "h":
          move(-1);
          break;
        case "ArrowDown":
        case "j":
          move(wall ? cols.current : 1);
          break;
        case "ArrowUp":
        case "k":
          move(wall ? -cols.current : -1);
          break;
        case "PageDown":
          move(10);
          break;
        case "PageUp":
          move(-10);
          break;
        case "Home":
          setActive(0);
          break;
        case "End":
          setActive(Math.max(0, n - 1));
          break;
        case "Enter":
          void apply(active);
          break;
        case "Escape":
          void hideOverlay();
          break;
        case "r":
        case "R":
          say("Reloading…");
          void load(true).then((g) => g && say(`${g.items.length} images`));
          break;
        case "Tab":
          cycleStyle();
          break;
        case "s":
        case "S":
          void openSettings();
          break;
        default:
          return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, apply, cycleStyle, load, move, n, say, style]);

  // Mouse wheel / trackpad: one step per ~70 ms so it never runs away.
  const onWheel = (e: React.WheelEvent) => {
    if (style === "wall") return; // native scrolling
    const now = performance.now();
    if (now - wheelAt.current < 70) return;
    const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
    if (Math.abs(d) < 4) return;
    wheelAt.current = now;
    move(d > 0 ? 1 : -1);
  };

  const metrics = useMemo(
    () => (style === "wall" ? null : sizeFor(style, vp.w, vp.h)),
    [style, vp.w, vp.h],
  );

  const cards = useMemo(() => {
    if (style === "wall" || !metrics) return null;
    const reach = REACH[style];
    const from = Math.max(0, active - reach);
    const to = Math.min(n - 1, active + reach);
    const out = [];
    for (let i = from; i <= to; i++) {
      const look = cardLook(style, i - active, metrics);
      if (!look.visible) continue;
      out.push(
        <Card
          key={items[i].id}
          item={items[i]}
          active={i === active}
          style={look.style}
          dim={look.dim}
          onClick={(isActive) => (isActive ? void apply(i) : setActive(i))}
          onDoubleClick={() => void apply(i)}
        />,
      );
    }
    return out;
  }, [style, metrics, active, n, items, apply]);

  const current = items[active];

  return (
    <div
      className={"stage" + (shown ? " shown" : "")}
      onWheel={onWheel}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) void hideOverlay();
      }}
      style={
        metrics
          ? ({ "--cw": `${metrics.cw}px`, "--ch": `${metrics.ch}px` } as React.CSSProperties)
          : undefined
      }
    >
      {n === 0 ? (
        <div className="empty">
          <h1>{gallery?.folder ? "No images in this folder" : "Choose your wallpaper folder"}</h1>
          <p>
            {gallery?.folder
              ? gallery.folder
              : "Pick a folder and WallFlip will show its images here."}
          </p>
          <button onClick={() => void openSettings()}>Open settings</button>
        </div>
      ) : style === "wall" ? (
        <Wall
          items={items}
          active={active}
          onSelect={setActive}
          onApply={(i) => void apply(i)}
          onColumns={(c) => (cols.current = c)}
        />
      ) : (
        <div className={"scene " + style}>{cards}</div>
      )}

      {n > 0 && current && (
        <div className="hud">
          <div className="title">{prettyName(current.name)}</div>
          <div className="meta">
            {active + 1} / {n} · {STYLE_LABEL[style]}
          </div>
          <div className="hints">
            <kbd>←</kbd>
            <kbd>→</kbd> browse <span>·</span> <kbd>Enter</kbd> apply <span>·</span> <kbd>Tab</kbd> style{" "}
            <span>·</span> <kbd>R</kbd> reload <span>·</span> <kbd>S</kbd> settings <span>·</span>{" "}
            <kbd>Esc</kbd> close
          </div>
        </div>
      )}

      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}

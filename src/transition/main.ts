// Runs wallpaper-change animations full-screen, one job at a time, then reports back to Rust.
// The page stays loaded between jobs (Rust keeps the window for a while), so a job starts instantly.
// Only transform / opacity / clip-path are animated, so it runs on the GPU compositor.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./transition.css";

interface Job {
  from: string; // hex-encoded paths, served by the Rust `wall` protocol
  to: string;
  kind: string;
  ms: number;
  fit: string;
  w: number; // picture size cap in physical px (0 = native size)
  h: number;
  mw: number; // whole monitor, physical px
  mh: number;
  ox: number; // offset of this window inside the monitor, physical px
  oy: number;
}

const stage = document.getElementById("stage") as HTMLDivElement;

// per-job state, set by run()
let kind = "fade";
let ms = 900;
let SW = window.innerWidth;
let SH = window.innerHeight;

const EASE = "cubic-bezier(0.65, 0, 0.35, 1)";
const OUT = "cubic-bezier(0.22, 1, 0.36, 1)";

function layer(src: string): HTMLDivElement {
  const d = document.createElement("div");
  d.className = "layer";
  d.style.backgroundImage = `url("${src}")`;
  return d;
}

/** A clipping box showing the `to` picture, for effects made of many small pieces. */
function cell(src: string, x: number, y: number, w: number, h: number): HTMLDivElement {
  const c = document.createElement("div");
  c.className = "cell";
  Object.assign(c.style, { left: `${x}px`, top: `${y}px`, width: `${w}px`, height: `${h}px` });
  const l = layer(src);
  Object.assign(l.style, {
    left: `${-x}px`,
    top: `${-y}px`,
    width: `${SW}px`,
    height: `${SH}px`,
  });
  c.appendChild(l);
  return c;
}

type Anim = Animation;
const opts = (duration: number, easing = EASE, delay = 0): KeyframeAnimationOptions => ({
  duration,
  easing,
  delay,
  fill: "both",
});

/** Builds the animation. `from` is already on stage; effects add/animate pieces of `to` on top. */
function build(from: HTMLElement, to: HTMLElement, src: string): Anim[] {
  to.style.opacity = ""; // pre-painted invisibly so the GPU already holds the texture
  const anims: Anim[] = [];
  if (kind === "blinds" || kind === "tiles") to.remove(); // these draw their own pieces
  const add = (el: HTMLElement, frames: Keyframe[], o: KeyframeAnimationOptions) => {
    anims.push(el.animate(frames, o));
  };

  switch (kind) {
    case "fade":
      stage.appendChild(to);
      add(to, [{ opacity: 0 }, { opacity: 1 }], opts(ms, "ease-in-out"));
      break;
    case "slide":
      stage.appendChild(to);
      add(to, [{ transform: "translateX(100%)" }, { transform: "translateX(0)" }], opts(ms));
      break;
    case "push":
      stage.appendChild(to);
      add(from, [{ transform: "translateX(0)" }, { transform: "translateX(-100%)" }], opts(ms));
      add(to, [{ transform: "translateX(100%)" }, { transform: "translateX(0)" }], opts(ms));
      break;
    case "wipe":
      stage.appendChild(to);
      add(to, [{ clipPath: "inset(0 100% 0 0)" }, { clipPath: "inset(0 0 0 0)" }], opts(ms));
      break;
    case "diagonal":
      stage.appendChild(to);
      add(
        to,
        [{ clipPath: "polygon(0 0, 0 0, 0 0)" }, { clipPath: "polygon(0 0, 210% 0, 0 210%)" }],
        opts(ms),
      );
      break;
    case "circle":
      stage.appendChild(to);
      add(to, [{ clipPath: "circle(0% at 50% 50%)" }, { clipPath: "circle(75% at 50% 50%)" }], opts(ms));
      break;
    case "diamond":
      stage.appendChild(to);
      add(
        to,
        [
          { clipPath: "polygon(50% 50%, 50% 50%, 50% 50%, 50% 50%)" },
          { clipPath: "polygon(50% -60%, 160% 50%, 50% 160%, -60% 50%)" },
        ],
        opts(ms),
      );
      break;
    case "zoom":
      stage.appendChild(to);
      add(
        to,
        [
          { opacity: 0, transform: "scale(1.3)" },
          { opacity: 1, transform: "scale(1)" },
        ],
        opts(ms, OUT),
      );
      break;
    case "zoomout": {
      // old picture bursts towards you and dissolves, revealing the new one beneath
      stage.insertBefore(to, from);
      add(
        from,
        [
          { opacity: 1, transform: "scale(1)" },
          { opacity: 0, transform: "scale(1.4)" },
        ],
        opts(ms),
      );
      break;
    }
    case "blur":
      stage.appendChild(to);
      add(
        to,
        [
          { opacity: 0, filter: "blur(28px)" },
          { opacity: 1, filter: "blur(0px)" },
        ],
        opts(ms, "ease-in-out"),
      );
      break;
    case "blinds": {
      const n = 12;
      const w = SW / n;
      const per = ms * 0.55;
      for (let i = 0; i < n; i++) {
        const c = cell(src, Math.floor(i * w), 0, Math.ceil(w) + 1, SH);
        stage.appendChild(c);
        add(
          c,
          [{ clipPath: "inset(0 100% 0 0)" }, { clipPath: "inset(0 0 0 0)" }],
          opts(per, EASE, (i / (n - 1)) * (ms - per)),
        );
      }
      break;
    }
    case "tiles": {
      const cols = 16;
      const size = Math.ceil(SW / cols);
      const rows = Math.ceil(SH / size);
      const per = ms * 0.4;
      for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
          const el = cell(src, c * size, r * size, size + 1, size + 1);
          stage.appendChild(el);
          add(
            el,
            [
              { opacity: 0, transform: "scale(0.4)" },
              { opacity: 1, transform: "scale(1)" },
            ],
            opts(per, OUT, Math.random() * (ms - per)),
          );
        }
      }
      break;
    }
    case "flip": {
      // two half-turns: the old picture swings away, the new one swings in
      const half = ms / 2;
      stage.appendChild(to);
      for (const el of [from, to]) el.style.backfaceVisibility = "hidden";
      add(
        from,
        [{ transform: "rotateY(0deg)" }, { transform: "rotateY(90deg)" }],
        opts(half, "cubic-bezier(0.55, 0, 1, 0.45)"),
      );
      add(
        to,
        [{ transform: "rotateY(-90deg)" }, { transform: "rotateY(0deg)" }],
        opts(half, "cubic-bezier(0, 0.55, 0.45, 1)", half),
      );
      break;
    }
    default:
      stage.appendChild(to);
      add(to, [{ opacity: 0 }, { opacity: 1 }], opts(ms, "ease-in-out"));
  }
  return anims;
}

const held = new Map<string, HTMLImageElement>(); // keeps decoded pictures alive between jobs

async function preload(src: string): Promise<void> {
  if (held.has(src)) return;
  const img = new Image();
  img.src = src;
  await img.decode();
  held.set(src, img);
}

function deferred() {
  let fire!: () => void;
  const p = new Promise<void>((r) => (fire = r));
  return { p, fire };
}

let current: { show: ReturnType<typeof deferred>; go: ReturnType<typeof deferred> } | null = null;

async function run(job: Job) {
  const sig = { show: deferred(), go: deferred() };
  current = sig;
  kind = job.kind;
  ms = Math.max(150, job.ms || 900);
  const dpr = window.devicePixelRatio || 1;
  SW = job.mw / dpr;
  SH = job.mh / dpr;

  // The picture is laid out for the whole monitor exactly as Windows does; the stage is shifted so
  // this window shows its part of it.
  stage.className = `fit-${job.fit}`;
  Object.assign(stage.style, {
    left: `${-job.ox / dpr}px`,
    top: `${-job.oy / dpr}px`,
    width: `${SW}px`,
    height: `${SH}px`,
    right: "auto",
    bottom: "auto",
  });

  const url = (hex: string) => `${convertFileSrc(hex, "wall")}?w=${job.w}&h=${job.h}`;
  const fromUrl = url(job.from);
  const toUrl = url(job.to);
  try {
    await Promise.all([preload(fromUrl), preload(toUrl)]);
  } catch {
    await invoke("transition_failed");
    return;
  }
  for (const k of [...held.keys()]) if (k !== fromUrl && k !== toUrl) held.delete(k);

  const from = layer(fromUrl);
  const to = layer(toUrl);
  to.style.opacity = "0.001"; // painted (so uploaded to the GPU) but invisible
  stage.replaceChildren(from, to);
  await invoke("transition_ready");

  // Rust places and shows the window; a hidden page does not run animation frames, so paint only after
  // that. Two frames: the first paint of two 4K pictures must not happen during the animation.
  await sig.show.p;
  await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
  await invoke("transition_painted");
  await sig.go.p;

  const anims = build(from, to, toUrl);
  await Promise.race([
    Promise.all(anims.map((a) => a.finished)),
    new Promise((r) => setTimeout(r, ms + 1500)),
  ]).catch(() => undefined);
  await invoke("transition_done");
}

async function main() {
  await listen<Job>("transition-load", (e) => void run(e.payload));
  await listen("transition-show", () => current?.show.fire());
  await listen("transition-go", () => current?.go.fire());
  await listen("transition-reset", () => {
    current = null;
    stage.replaceChildren(); // free the textures while the window sleeps
    held.clear();
  });
  await invoke("transition_up");
}

void main();

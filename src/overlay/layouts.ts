import type { CSSProperties } from "react";
import type { StyleName } from "../lib/api";

/**
 * Pure layout maths for the 3D gallery styles. Every card is positioned with a
 * single `transform` (+ opacity) so the browser animates it on the GPU
 * compositor: no layout or paint work per frame.
 */

export interface Metrics {
  /** card width/height in CSS px */
  cw: number;
  ch: number;
}

export interface CardLook {
  style: CSSProperties;
  /** 0 = fully lit, 1 = black. Applied through a cheap overlay element. */
  dim: number;
  /** how many cards on each side are rendered */
  visible: boolean;
}

/** How many neighbours on each side get a DOM node. Everything else is culled. */
export const REACH: Record<Exclude<StyleName, "wall">, number> = {
  carousel: 6,
  depth: 7,
  fan: 8,
};

const sign = (n: number) => (n < 0 ? -1 : n > 0 ? 1 : 0);
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function sizeFor(style: StyleName, vw: number, vh: number): Metrics {
  const ratio = 16 / 9;
  let cw: number;
  switch (style) {
    case "fan":
      cw = clamp(Math.min(vw * 0.26, vh * 0.5 * ratio), 240, 640);
      break;
    case "depth":
      cw = clamp(Math.min(vw * 0.42, vh * 0.62 * ratio), 300, 900);
      break;
    default:
      cw = clamp(Math.min(vw * 0.4, vh * 0.6 * ratio), 300, 880);
  }
  return { cw: Math.round(cw), ch: Math.round(cw / ratio) };
}

export function cardLook(style: Exclude<StyleName, "wall">, offset: number, m: Metrics): CardLook {
  const a = Math.abs(offset);
  const s = sign(offset);
  const reach = REACH[style];
  const visible = a <= reach;
  // fade the outermost cards so they never pop in or out
  const edge = clamp((reach - a) / 2, 0, 1);

  switch (style) {
    case "carousel": {
      // flat strip: centre card big, neighbours shrink and tuck inwards
      const x = a === 0 ? 0 : s * (m.cw * 0.74 + (a - 1) * m.cw * 0.34);
      const scale = a === 0 ? 1 : Math.max(0.5, 0.74 - (a - 1) * 0.09);
      const rot = a === 0 ? 0 : -s * 24;
      const z = -a * 60;
      return {
        visible,
        dim: a === 0 ? 0 : Math.min(0.72, 0.28 + a * 0.1),
        style: {
          transform: `translate3d(${x}px,0,${z}px) rotateY(${rot}deg) scale(${scale})`,
          opacity: Math.min(1, edge * (a <= 2 ? 1 : 1 - (a - 2) * 0.12)),
          zIndex: 100 - a,
        },
      };
    }
    case "depth": {
      // coverflow-like: cards recede behind the active one
      const x = a === 0 ? 0 : s * (m.cw * 0.62 + (a - 1) * m.cw * 0.13);
      const z = a === 0 ? 0 : -(160 + a * 120);
      const rot = a === 0 ? 0 : -s * 38;
      return {
        visible,
        dim: a === 0 ? 0 : Math.min(0.8, 0.22 + a * 0.12),
        style: {
          transform: `translate3d(${x}px,${a === 0 ? 0 : 6}px,${z}px) rotateY(${rot}deg)`,
          opacity: edge,
          zIndex: 100 - a,
        },
      };
    }
    case "fan": {
      // a hand of cards fanned from a pivot far below the screen
      const rot = offset * 6.5;
      const lift = a === 0 ? -m.ch * 0.16 : 0;
      return {
        visible,
        dim: a === 0 ? 0 : Math.min(0.6, 0.12 + a * 0.07),
        style: {
          transformOrigin: "50% 210%",
          transform: `translate3d(0,${lift}px,0) rotate(${rot}deg) scale(${a === 0 ? 1.1 : 1})`,
          opacity: edge,
          zIndex: 100 - a,
        },
      };
    }
  }
}

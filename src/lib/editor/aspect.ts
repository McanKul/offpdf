/**
 * Aspect-ratio helpers for shape create/resize and inspector W×H edits.
 * Square/circle tools force 1:1; Shift toggles lock the same way Figma/Word do.
 */

import type { CssRect } from "./coords";
import type { ResizeHandle } from "./resizeRect";
import type { PdfRect } from "./types";

export function cssBoxFromPoints(
  a: { x: number; y: number },
  b: { x: number; y: number },
): CssRect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    w: Math.abs(b.x - a.x),
    h: Math.abs(b.y - a.y),
  };
}

/** 1:1 CSS box from a start corner toward the pointer (start corner stays put). */
export function constrainCssBox1to1(
  start: { x: number; y: number },
  cur: { x: number; y: number },
): CssRect {
  const dx = cur.x - start.x;
  const dy = cur.y - start.y;
  const size = Math.max(Math.abs(dx), Math.abs(dy));
  const left = dx < 0;
  const up = dy < 0;
  return {
    x: left ? start.x - size : start.x,
    y: up ? start.y - size : start.y,
    w: size,
    h: size,
  };
}

/** Resize a CSS rect from a corner while keeping start.w / start.h. */
export function resizeCssRectLocked(
  start: CssRect,
  handle: ResizeHandle,
  dx: number,
  dy: number,
): CssRect {
  const ratio = start.w / Math.max(start.h, 1e-6);
  let growW = 0;
  let growH = 0;
  switch (handle) {
    case "se":
      growW = dx;
      growH = dy;
      break;
    case "sw":
      growW = -dx;
      growH = dy;
      break;
    case "ne":
      growW = dx;
      growH = -dy;
      break;
    case "nw":
      growW = -dx;
      growH = -dy;
      break;
  }
  let w = Math.max(1, start.w + growW);
  let h = Math.max(1, start.h + growH);
  if (w / h > ratio) h = w / ratio;
  else w = h * ratio;
  w = Math.max(1, w);
  h = Math.max(1, h);
  switch (handle) {
    case "se":
      return { x: start.x, y: start.y, w, h };
    case "sw":
      return { x: start.x + start.w - w, y: start.y, w, h };
    case "ne":
      return { x: start.x, y: start.y + start.h - h, w, h };
    case "nw":
      return { x: start.x + start.w - w, y: start.y + start.h - h, w, h };
  }
}

/** Images lock by default; other kinds only when `keepAspect` is set. Shift toggles. */
export function aspectLocked(
  obj: { kind: string; keepAspect?: boolean },
  shiftKey: boolean,
): boolean {
  const on = obj.kind === "image" ? obj.keepAspect !== false : !!obj.keepAspect;
  return on !== shiftKey;
}

/** Apply a width and/or height edit, optionally preserving the current ratio. */
export function sizeWithAspect(
  rect: PdfRect,
  next: { w?: number; h?: number },
  lock: boolean,
): PdfRect {
  const ratio = rect.w / Math.max(rect.h, 1e-6);
  let w = next.w ?? rect.w;
  let h = next.h ?? rect.h;
  if (lock) {
    if (next.w != null && next.h == null) h = w / ratio;
    else if (next.h != null && next.w == null) w = h * ratio;
  }
  return { ...rect, w: Math.max(1, w), h: Math.max(1, h) };
}

export function isNearlySquare(rect: PdfRect): boolean {
  const m = Math.max(rect.w, rect.h, 1);
  return Math.abs(rect.w - rect.h) <= Math.max(0.75, 0.02 * m);
}

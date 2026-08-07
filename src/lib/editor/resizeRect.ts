/**
 * Resize a PDF-space rect from a CSS-named handle.
 *
 * PdfRect is unrotated PDF user space: (x, y) is the **lower-left**, y up.
 * Handle names are CSS/screen: n = top of the element, s = bottom.
 *
 *   CSS nw (top-left)    ↔ PDF upper-left  (x,     y+h)
 *   CSS ne (top-right)   ↔ PDF upper-right (x+w,   y+h)
 *   CSS sw (bottom-left) ↔ PDF lower-left  (x,     y)
 *   CSS se (bottom-right)↔ PDF lower-right (x+w,   y)
 *
 * `dx`/`dy` are pointer deltas already converted to PDF space.
 */

import { normalizePdfRect, type PdfRect } from "./types";
import type { CssRect } from "./coords";

export type ResizeHandle = "nw" | "ne" | "sw" | "se";

/**
 * Resize a CSS (top-left, y-down) rect from a named handle.
 * Use this under page /Rotate, then map the result with `viewportRectToPdf`.
 */
export function resizeCssRect(start: CssRect, handle: ResizeHandle, dx: number, dy: number): CssRect {
  let { x, y, w, h } = start;
  switch (handle) {
    case "se":
      w += dx;
      h += dy;
      break;
    case "sw":
      x += dx;
      w -= dx;
      h += dy;
      break;
    case "ne":
      w += dx;
      y += dy;
      h -= dy;
      break;
    case "nw":
      x += dx;
      y += dy;
      w -= dx;
      h -= dy;
      break;
  }
  if (w < 0) {
    x += w;
    w = -w;
  }
  if (h < 0) {
    y += h;
    h = -h;
  }
  if (w < 1) w = 1;
  if (h < 1) h = 1;
  return { x, y, w, h };
}

export function resizePdfRect(
  start: PdfRect,
  handle: ResizeHandle,
  dx: number,
  dy: number,
): PdfRect {
  // Anchor = opposite corner stays fixed.
  switch (handle) {
    case "se": {
      // Move lower-right; keep upper-left (x, y+h) fixed.
      return normalizePdfRect({
        x: start.x,
        y: start.y + dy,
        w: start.w + dx,
        h: start.h - dy,
      });
    }
    case "sw": {
      // Move lower-left; keep upper-right (x+w, y+h) fixed.
      return normalizePdfRect({
        x: start.x + dx,
        y: start.y + dy,
        w: start.w - dx,
        h: start.h - dy,
      });
    }
    case "ne": {
      // Move upper-right; keep lower-left (x, y) fixed.
      return normalizePdfRect({
        x: start.x,
        y: start.y,
        w: start.w + dx,
        h: start.h + dy,
      });
    }
    case "nw": {
      // Move upper-left; keep lower-right (x+w, y) fixed.
      return normalizePdfRect({
        x: start.x + dx,
        y: start.y,
        w: start.w - dx,
        h: start.h + dy,
      });
    }
  }
}

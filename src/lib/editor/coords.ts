/**
 * Viewport CSS pixel ↔ unrotated PDF point transforms for the editor canvas.
 *
 * Stored geometry is always in unrotated PDF user space (origin = lower-left of
 * the visible page box). /Rotate only affects display mapping.
 *
 * Pure functions — no DOM, no pdf.js — so unit tests stay hermetic.
 */

import type { PageGeometry, PageRotation, PdfRect, Point } from "./types";

/** CSS-space axis-aligned rect (origin top-left of the page element). */
export interface CssRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface ViewportMapping {
  /** CSS pixel width of the rendered page element. */
  cssWidth: number;
  /** CSS pixel height of the rendered page element. */
  cssHeight: number;
  geometry: PageGeometry;
}

/** Displayed page size in PDF points after applying /Rotate (still BL origin). */
export function displayedSize(geometry: PageGeometry): { w: number; h: number } {
  const { w, h } = geometry.box;
  const r = geometry.rotate;
  if (r === 90 || r === 270) return { w: h, h: w };
  return { w, h };
}

/**
 * Map a point relative to the page box (origin BL of box) into displayed
 * page coordinates (origin BL of the rotated display, size = displayedSize).
 *
 * PDF /Rotate is clockwise for display.
 */
export function unrotatedToDisplay(
  rx: number,
  ry: number,
  boxW: number,
  boxH: number,
  rotate: PageRotation,
): Point {
  switch (rotate) {
    case 0:
      return { x: rx, y: ry };
    case 90:
      // CW 90: (rx, ry) → (ry, boxW - rx); display size (boxH, boxW)
      return { x: ry, y: boxW - rx };
    case 180:
      return { x: boxW - rx, y: boxH - ry };
    case 270:
      // CW 270: (rx, ry) → (boxH - ry, rx); display size (boxH, boxW)
      return { x: boxH - ry, y: rx };
  }
}

/** Inverse of unrotatedToDisplay. */
export function displayToUnrotated(
  dx: number,
  dy: number,
  boxW: number,
  boxH: number,
  rotate: PageRotation,
): Point {
  switch (rotate) {
    case 0:
      return { x: dx, y: dy };
    case 90:
      // inverse of (ry, boxW - rx)
      return { x: boxW - dy, y: dx };
    case 180:
      return { x: boxW - dx, y: boxH - dy };
    case 270:
      // inverse of (boxH - ry, rx)
      return { x: dy, y: boxH - dx };
  }
}

/** PDF point (absolute user space) → CSS pixel (top-left origin of page el). */
export function pdfToViewport(pt: Point, m: ViewportMapping): Point {
  const { box, rotate } = m.geometry;
  const rx = pt.x - box.x;
  const ry = pt.y - box.y;
  const disp = unrotatedToDisplay(rx, ry, box.w, box.h, rotate);
  const { w: dispW, h: dispH } = displayedSize(m.geometry);
  if (dispW <= 0 || dispH <= 0 || m.cssWidth <= 0 || m.cssHeight <= 0) {
    return { x: 0, y: 0 };
  }
  return {
    x: (disp.x / dispW) * m.cssWidth,
    y: ((dispH - disp.y) / dispH) * m.cssHeight,
  };
}

/** CSS pixel → PDF point (absolute user space, unrotated). */
export function viewportToPdf(px: Point, m: ViewportMapping): Point {
  const { box, rotate } = m.geometry;
  const { w: dispW, h: dispH } = displayedSize(m.geometry);
  if (dispW <= 0 || dispH <= 0 || m.cssWidth <= 0 || m.cssHeight <= 0) {
    return { x: box.x, y: box.y };
  }
  const dx = (px.x / m.cssWidth) * dispW;
  const dy = dispH - (px.y / m.cssHeight) * dispH;
  const rel = displayToUnrotated(dx, dy, box.w, box.h, rotate);
  return { x: box.x + rel.x, y: box.y + rel.y };
}

/**
 * Map a PDF rect (axis-aligned in unrotated space) to a CSS AABB.
 * Corners are transformed individually so 90/270 rotations swap visual axes.
 */
export function pdfRectToViewport(rect: PdfRect, m: ViewportMapping): CssRect {
  const corners: Point[] = [
    { x: rect.x, y: rect.y },
    { x: rect.x + rect.w, y: rect.y },
    { x: rect.x + rect.w, y: rect.y + rect.h },
    { x: rect.x, y: rect.y + rect.h },
  ];
  const mapped = corners.map((c) => pdfToViewport(c, m));
  const xs = mapped.map((p) => p.x);
  const ys = mapped.map((p) => p.y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
}

/**
 * Map a CSS rect (axis-aligned) to a PDF AABB in unrotated space.
 * Useful when creating objects from a drag on screen.
 */
export function viewportRectToPdf(rect: CssRect, m: ViewportMapping): PdfRect {
  const corners: Point[] = [
    { x: rect.x, y: rect.y },
    { x: rect.x + rect.w, y: rect.y },
    { x: rect.x + rect.w, y: rect.y + rect.h },
    { x: rect.x, y: rect.y + rect.h },
  ];
  const mapped = corners.map((c) => viewportToPdf(c, m));
  const xs = mapped.map((p) => p.x);
  const ys = mapped.map((p) => p.y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
}

/** Build a mapping for tests and layout callbacks. */
export function makeMapping(
  geometry: PageGeometry,
  cssWidth: number,
  cssHeight: number,
): ViewportMapping {
  return { cssWidth, cssHeight, geometry };
}

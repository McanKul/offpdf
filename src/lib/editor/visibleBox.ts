/**
 * Visible page box = CropBox ∩ MediaBox (else MediaBox).
 * Must match Rust `crop::visible_box` so preview and overlay export agree.
 */

import type { PageBox } from "./types";

/** PDF box as [x0, y0, x1, y1] in unrotated user space. */
export type PdfBoxQuad = [number, number, number, number];

export function quadToBox(q: PdfBoxQuad): PageBox {
  const x0 = Math.min(q[0], q[2]);
  const y0 = Math.min(q[1], q[3]);
  const x1 = Math.max(q[0], q[2]);
  const y1 = Math.max(q[1], q[3]);
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

export function boxToQuad(b: PageBox): PdfBoxQuad {
  return [b.x, b.y, b.x + b.w, b.y + b.h];
}

/** Intersect crop with media; fall back to media if the intersection is tiny. */
export function visiblePageBox(media: PdfBoxQuad, crop?: PdfBoxQuad | null): PageBox {
  const mb = quadToBox(media);
  if (!crop) return mb;
  const cb = quadToBox(crop);
  const x0 = Math.max(mb.x, cb.x);
  const y0 = Math.max(mb.y, cb.y);
  const x1 = Math.min(mb.x + mb.w, cb.x + cb.w);
  const y1 = Math.min(mb.y + mb.h, cb.y + cb.h);
  if (x1 - x0 > 1 && y1 - y0 > 1) {
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }
  return mb;
}

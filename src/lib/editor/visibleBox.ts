/**
 * Visible page box = CropBox ∩ MediaBox (else MediaBox).
 * Matches Rust `crop::visible_box`. Preview uses pdf.js `page.view` (same
 * window). Rust also reads qpdf's native alignment box (page Trim → Crop →
 * Media) to decide whether the temporary export pages need normalization.
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

function intersectOrMedia(inner: PageBox, mb: PageBox): PageBox {
  const x0 = Math.max(mb.x, inner.x);
  const y0 = Math.max(mb.y, inner.y);
  const x1 = Math.min(mb.x + mb.w, inner.x + inner.w);
  const y1 = Math.min(mb.y + mb.h, inner.y + inner.h);
  if (x1 - x0 > 1 && y1 - y0 > 1) {
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }
  return mb;
}

/** Intersect crop with media; fall back to media if the intersection is tiny. */
export function visiblePageBox(media: PdfBoxQuad, crop?: PdfBoxQuad | null): PageBox {
  const mb = quadToBox(media);
  if (!crop) return mb;
  return intersectOrMedia(quadToBox(crop), mb);
}

/**
 * qpdf's native overlay alignment model: raw page TrimBox (not inherited and
 * not clipped to Media) → CropBox → MediaBox. Preview does not see TrimBox.
 * Export normalizes mismatched working-page boxes to the visible box before
 * composition, then restores the source boxes.
 */
export function alignPageBox(
  media: PdfBoxQuad,
  crop?: PdfBoxQuad | null,
  trim?: PdfBoxQuad | null,
): PageBox {
  if (trim) return quadToBox(trim);
  if (crop) return quadToBox(crop);
  return quadToBox(media);
}

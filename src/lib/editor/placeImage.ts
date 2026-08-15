/**
 * Place an image in *displayed* page space (after /Rotate), then map to PDF.
 * Sizing against the unrotated box letterboxes on 90/270 preview and stretches
 * on export — always build the CSS rect from displayedSize.
 */

import { displayedSize, makeMapping, viewportRectToPdf, type CssRect } from "./coords";
import type { PageGeometry, PdfRect } from "./types";

export function imageCssRect(
  natW: number,
  natH: number,
  cssWidth: number,
  cssHeight: number,
  dispW: number,
  dispH: number,
  atCss?: { x: number; y: number },
): CssRect {
  const maxW = dispW * 0.45;
  const scale = natW > 0 ? Math.min(1, maxW / natW) : 1;
  const wPt = Math.max(24, natW * scale);
  const hPt = Math.max(24, natH * scale);
  const cssW = dispW > 0 ? (wPt / dispW) * cssWidth : cssWidth * 0.45;
  const cssH = dispH > 0 ? (hPt / dispH) * cssHeight : cssHeight * 0.45;
  if (atCss) {
    return { x: atCss.x - cssW / 2, y: atCss.y - cssH / 2, w: cssW, h: cssH };
  }
  return {
    x: (cssWidth - cssW) / 2,
    y: (cssHeight - cssH) / 2,
    w: cssW,
    h: cssH,
  };
}

export function placeImagePdfRect(
  geometry: PageGeometry,
  cssWidth: number,
  cssHeight: number,
  natW: number,
  natH: number,
  atCss?: { x: number; y: number },
): PdfRect {
  const disp = displayedSize(geometry);
  const css = imageCssRect(natW, natH, cssWidth, cssHeight, disp.w, disp.h, atCss);
  return viewportRectToPdf(css, makeMapping(geometry, cssWidth, cssHeight));
}

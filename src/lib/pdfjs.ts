/** pdf.js setup for true-vector page rendering in the viewer. The worker and
 * font/cmap assets are bundled locally (no network). Only a single extracted
 * page's bytes are ever loaded here — never the whole document. */
import * as pdfjsLib from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl;

/** Shared options: local cmaps + standard fonts, eval disabled (CSP-safe). */
export const PDF_OPTS = {
  cMapUrl: "/pdfjs/cmaps/",
  cMapPacked: true,
  standardFontDataUrl: "/pdfjs/standard_fonts/",
  isEvalSupported: false,
} as const;

export { pdfjsLib };

/** Decode a base64 string (one-page PDF) to bytes for pdf.js. */
export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

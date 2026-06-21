/** Paper sizes and poster-tiling math, shared by the Poster tool and its
 * live estimate. Pure functions only (no pdf.js / Tauri) so they stay testable.
 * All dimensions are in PostScript points unless a name ends in `Mm`. */

export const PT_PER_MM = 2.834645669;
export const MM_PER_PT = 1 / PT_PER_MM;

export type PaperId = "a4" | "a3" | "letter";
export type Orientation = "auto" | "portrait" | "landscape";

interface PaperSize {
  id: PaperId;
  label: string;
  wMm: number;
  hMm: number;
}

export const PAPERS: PaperSize[] = [
  { id: "a4", label: "A4 (210 × 297 mm)", wMm: 210, hMm: 297 },
  { id: "a3", label: "A3 (297 × 420 mm)", wMm: 297, hMm: 420 },
  { id: "letter", label: "Letter (8.5 × 11 in)", wMm: 215.9, hMm: 279.4 },
];

const PAPER_BY_ID = new Map(PAPERS.map((p) => [p.id, p]));

/** Tile size in points for a paper id + orientation (portrait = tall). */
export function paperPt(id: PaperId, portrait: boolean): { w: number; h: number } {
  const p = PAPER_BY_ID.get(id) ?? PAPERS[0];
  const w = (portrait ? p.wMm : p.hMm) * PT_PER_MM;
  const h = (portrait ? p.hMm : p.wMm) * PT_PER_MM;
  return { w, h };
}

/** Number of tiles spanning `total`, given a tile length and overlap. */
export function tileCount(total: number, tile: number, overlap: number): number {
  if (total <= tile + 0.5) return 1;
  const step = Math.max(tile - overlap, 1);
  return Math.ceil((total - tile) / step) + 1;
}

export interface Grid {
  cols: number;
  rows: number;
  count: number;
  /** Resolved tile size in points (after auto-orientation). */
  tileW: number;
  tileH: number;
}

/** Grid for a page (pts) using a paper id + orientation choice + overlap (pts). */
export function gridFor(
  pageW: number,
  pageH: number,
  paper: PaperId,
  orientation: Orientation,
  overlap: number,
): Grid {
  const build = (portrait: boolean): Grid => {
    const { w, h } = paperPt(paper, portrait);
    const cols = Math.max(1, tileCount(pageW, w, overlap));
    const rows = Math.max(1, tileCount(pageH, h, overlap));
    return { cols, rows, count: cols * rows, tileW: w, tileH: h };
  };
  if (orientation === "portrait") return build(true);
  if (orientation === "landscape") return build(false);
  const a = build(true);
  const b = build(false);
  return b.count < a.count ? b : a;
}

const ISO_A: Record<string, [number, number]> = {
  A0: [841, 1189],
  A1: [594, 841],
  A2: [420, 594],
  A3: [297, 420],
  A4: [210, 297],
  A5: [148, 210],
};

/** Human label for a page size in points, naming ISO A-series when it matches. */
export function describeSize(wPt: number, hPt: number): string {
  const wMm = wPt * MM_PER_PT;
  const hMm = hPt * MM_PER_PT;
  const [loMm, hiMm] = wMm <= hMm ? [wMm, hMm] : [hMm, wMm];
  for (const [name, [a, b]] of Object.entries(ISO_A)) {
    if (Math.abs(loMm - a) <= 6 && Math.abs(hiMm - b) <= 6) {
      return `${name} (${Math.round(wMm)} × ${Math.round(hMm)} mm)`;
    }
  }
  return `${Math.round(wMm)} × ${Math.round(hMm)} mm`;
}

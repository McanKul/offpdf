/**
 * Typed edit model for the visual PDF editor canvas (issue #6).
 *
 * Objects are stored in unrotated PDF user space (points). Source PDF bytes and
 * file paths live outside this document — only geometry and draft objects do.
 *
 * See EDIT_MODEL.md for the full contract.
 */

/** Visible page box in unrotated PDF user space (points). Origin is the
 * lower-left of the effective crop/media box. */
export interface PageBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type PageRotation = 0 | 90 | 180 | 270;

export interface PageGeometry {
  /** Effective crop/media box used for placement. */
  box: PageBox;
  /** Page /Rotate (0 | 90 | 180 | 270). */
  rotate: PageRotation;
  /** 0-based page index within this editor session. */
  pageIndex: number;
}

/** Axis-aligned box in unrotated PDF page space (points, origin bottom-left of
 * the page box). */
export interface PdfRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Point {
  x: number;
  y: number;
}

/** Generic draft object kinds — later tools (#7–#10) specialize payloads. */
export type EditObjectKind =
  | "rect"
  | "ellipse"
  | "text"
  | "image"
  | "path"
  | "ink"
  | "redaction";

export interface EditObjectBase {
  id: string;
  kind: EditObjectKind;
  pageIndex: number;
  /** Axis-aligned box in unrotated PDF page space (points). */
  rect: PdfRect;
  /** Optional object rotation around rect center (degrees). MVP stores 0. */
  objectRotate?: number;
  locked?: boolean;
}

/** MVP rect payload — enough to exercise the canvas before #7 content. */
export interface RectObject extends EditObjectBase {
  kind: "rect";
  fill?: string;
  stroke?: string;
  opacity?: number;
}

/** Union widens as tools land; #6 only creates rect drafts. */
export type EditObject = RectObject;

export interface EditDocument {
  version: 1;
  objects: EditObject[];
  selectedIds: string[];
}

export function createEmptyDocument(): EditDocument {
  return { version: 1, objects: [], selectedIds: [] };
}

/** Normalize a rect so w/h are positive and at least `minSize`. */
export function normalizePdfRect(rect: PdfRect, minSize = 1): PdfRect {
  let { x, y, w, h } = rect;
  if (w < 0) {
    x += w;
    w = -w;
  }
  if (h < 0) {
    y += h;
    h = -h;
  }
  if (w < minSize) w = minSize;
  if (h < minSize) h = minSize;
  return { x, y, w, h };
}

export function isPageRotation(n: number): n is PageRotation {
  return n === 0 || n === 90 || n === 180 || n === 270;
}

/** Coerce any angle to 0 | 90 | 180 | 270. */
export function normalizePageRotation(degrees: number): PageRotation {
  const r = ((Math.round(degrees / 90) * 90) % 360 + 360) % 360;
  if (r === 90 || r === 180 || r === 270) return r;
  return 0;
}

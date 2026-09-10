/**
 * Typed edit model for the visual PDF editor canvas (issue #6).
 *
 * Objects are stored in unrotated PDF user space (points). Source PDF bytes and
 * file paths live outside this document — only geometry and draft objects do.
 *
 * See EDIT_MODEL.md for the full contract.
 */

/** Axis-aligned box in unrotated PDF user space (points). `geometry.box` is the
 * pdf.js visible window and the export mapping window. */
export interface PageBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type PageRotation = 0 | 90 | 180 | 270;

export interface PageGeometry {
  /** Visible box (pdf.js `view` / Crop ∩ Media) used by preview and export. */
  box: PageBox;
  /** Page /UserUnit (default 1). Preview CSS already includes this via pdf.js. */
  userUnit?: number;
  /** Page /Rotate (0 | 90 | 180 | 270). */
  rotate: PageRotation;
  /** 0-based page index within this editor session. */
  pageIndex: number;
}

/** Axis-aligned box in absolute unrotated PDF user space (points). Preview and
 * export mapping subtract `geometry.box`. */
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

export type EditObjectKind =
  | "rect"
  | "roundRect"
  | "ellipse"
  | "triangle"
  | "star"
  | "hexagon"
  | "bubble"
  | "arrow"
  | "text"
  | "image"
  | "line"
  | "ink"
  | "link"
  | "note"
  | "highlight"
  | "underline"
  | "strikeout"
  | "markupInk"
  | "redact";

export type ClosedShapeKind =
  | "rect"
  | "roundRect"
  | "ellipse"
  | "triangle"
  | "star"
  | "hexagon"
  | "bubble"
  | "arrow";

const CLOSED = new Set<string>([
  "rect",
  "roundRect",
  "ellipse",
  "triangle",
  "star",
  "hexagon",
  "bubble",
  "arrow",
]);

export function isClosedShape(kind: string): kind is ClosedShapeKind {
  return CLOSED.has(kind);
}

export function isClosedShapeObject(
  o: EditObject,
): o is
  | RectObject
  | RoundRectObject
  | EllipseObject
  | TriangleObject
  | StarObject
  | HexagonObject
  | BubbleObject
  | ArrowObject {
  return isClosedShape(o.kind);
}

const MARKUP = new Set<string>(["note", "highlight", "underline", "strikeout", "markupInk"]);

export function isMarkupObject(
  o: EditObject,
): o is NoteObject | HighlightObject | UnderlineObject | StrikeoutObject | MarkupInkObject {
  return MARKUP.has(o.kind);
}

export type TextAlign = "left" | "center" | "right";

export interface EditObjectBase {
  id: string;
  kind: EditObjectKind;
  pageIndex: number;
  /** Axis-aligned box in unrotated PDF page space (points). */
  rect: PdfRect;
  /** Optional object rotation around rect center (degrees). MVP stores 0. */
  objectRotate?: number;
  locked?: boolean;
  /** When true, resize and W×H edits keep the current width/height ratio. */
  keepAspect?: boolean;
}

export interface ShapeStyle {
  fill?: string;
  stroke?: string;
  strokeWidth?: number;
  opacity?: number;
}

export interface RectObject extends EditObjectBase, ShapeStyle {
  kind: "rect";
}

export interface EllipseObject extends EditObjectBase, ShapeStyle {
  kind: "ellipse";
}

export interface TriangleObject extends EditObjectBase, ShapeStyle {
  kind: "triangle";
}

export interface StarObject extends EditObjectBase, ShapeStyle {
  kind: "star";
}

export interface RoundRectObject extends EditObjectBase, ShapeStyle {
  kind: "roundRect";
}

export interface HexagonObject extends EditObjectBase, ShapeStyle {
  kind: "hexagon";
}

export interface BubbleObject extends EditObjectBase, ShapeStyle {
  kind: "bubble";
}

export interface ArrowObject extends EditObjectBase, ShapeStyle {
  kind: "arrow";
}

export interface TextObject extends EditObjectBase {
  kind: "text";
  content: string;
  fontSize: number;
  color?: string;
  align?: TextAlign;
  opacity?: number;
}

export interface ImageObject extends EditObjectBase {
  kind: "image";
  /** Absolute filesystem path. Export reads this; bytes stay out of React. */
  path: string;
  opacity?: number;
  /** Session-only preview data URL; stripped before IPC. */
  previewUrl?: string;
}

export interface LineObject extends EditObjectBase {
  kind: "line";
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  stroke?: string;
  strokeWidth?: number;
  opacity?: number;
}

export interface InkObject extends EditObjectBase {
  kind: "ink";
  points: Point[];
  stroke?: string;
  strokeWidth?: number;
  opacity?: number;
}

/** URI or in-document GoTo. Same unrotated `rect` space as overlay stamps. */
export type LinkAction =
  | { type: "uri"; uri: string }
  | { type: "goto"; destPageIndex: number };

export interface LinkObject extends EditObjectBase {
  kind: "link";
  action: LinkAction;
}

/** Inspector author / color / optional comment for session markup annots. */
export interface MarkupAnnotFields {
  /** Inspector string only. No OS user / "OffPDF" default. */
  author: string;
  color?: string;
  /** Optional comment text written as `/Contents` when set. */
  comment?: string;
}

export interface NoteObject extends EditObjectBase, MarkupAnnotFields {
  kind: "note";
}

export interface HighlightObject extends EditObjectBase, MarkupAnnotFields {
  kind: "highlight";
  /** Unrotated user-space QuadPoints (8×n). Winding is not frozen. */
  quads: number[];
}

export interface UnderlineObject extends EditObjectBase, MarkupAnnotFields {
  kind: "underline";
  quads: number[];
}

export interface StrikeoutObject extends EditObjectBase, MarkupAnnotFields {
  kind: "strikeout";
  quads: number[];
}

export interface MarkupInkObject extends EditObjectBase, MarkupAnnotFields {
  kind: "markupInk";
  /** Strokes in unrotated user space (`/InkList`). Not overlay Draw `ink`. */
  strokes: Point[][];
}

/** Permanent redaction region. Not an overlay stamp; Save rasterizes the page. */
export interface RedactObject extends EditObjectBase {
  kind: "redact";
  fill?: string;
  /** Optional replacement text burned into the raster. Omitted unless typed. */
  label?: string;
}

export type EditObject =
  | RectObject
  | RoundRectObject
  | EllipseObject
  | TriangleObject
  | StarObject
  | HexagonObject
  | BubbleObject
  | ArrowObject
  | TextObject
  | ImageObject
  | LineObject
  | InkObject
  | LinkObject
  | NoteObject
  | HighlightObject
  | UnderlineObject
  | StrikeoutObject
  | MarkupInkObject
  | RedactObject;

/** Bounds of a line segment. */
export function lineBounds(x1: number, y1: number, x2: number, y2: number): PdfRect {
  const x = Math.min(x1, x2);
  const y = Math.min(y1, y2);
  return normalizePdfRect({ x, y, w: Math.abs(x2 - x1), h: Math.abs(y2 - y1) });
}

/** Bounds of an ink stroke. */
export function pointsBounds(points: Point[]): PdfRect {
  if (points.length === 0) return { x: 0, y: 0, w: 1, h: 1 };
  let minX = points[0].x;
  let minY = points[0].y;
  let maxX = points[0].x;
  let maxY = points[0].y;
  for (const p of points) {
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  return normalizePdfRect({ x: minX, y: minY, w: maxX - minX, h: maxY - minY });
}

/** Map points from one AABB into another (resize line/ink with the box). */
export function mapPointsToRect(points: Point[], from: PdfRect, to: PdfRect): Point[] {
  const sx = from.w !== 0 ? to.w / from.w : 1;
  const sy = from.h !== 0 ? to.h / from.h : 1;
  return points.map((p) => ({
    x: to.x + (p.x - from.x) * sx,
    y: to.y + (p.y - from.y) * sy,
  }));
}

export interface EditDocument {
  version: 1;
  objects: EditObject[];
  selectedIds: string[];
}

/** Fillable AcroForm control from `list_pdf_form_fields` (lopdf walker). */
export type FormFieldKind = "text" | "checkbox" | "radio" | "combo" | "list";

export interface FormField {
  name: string;
  kind: FormFieldKind;
  pageIndex: number | null;
  rect: PdfRect | null;
  value: string | null;
  exportValues: string[];
  choices: string[];
  readOnly: boolean;
  hidden: boolean;
  maxLen: number | null;
  multiline: boolean;
  comboEdit: boolean;
}

export interface FormValue {
  name: string;
  value: string;
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

/**
 * Align selected editor objects in displayed page space.
 *
 * The visible page box and every object rect are stored in unrotated PDF user
 * space. Users align what they see, so bounds are mapped through `/Rotate`
 * first; the resulting translation is mapped back to PDF space before the
 * objects are offset. CropBox offsets disappear with the same subtraction the
 * preview uses.
 *
 * Multi-selection aligns to the selection bounds; a single object aligns to
 * the visible page box. Locked objects and objects on other pages never move.
 */

import { displayedSize, unrotatedToDisplay } from "./coords";
import { offsetObject } from "./serialize";
import type { EditObject, PageGeometry, PdfRect, PageRotation } from "./types";

export const ALIGN_COMMANDS = [
  "left",
  "horizontalCenter",
  "right",
  "top",
  "verticalCenter",
  "bottom",
] as const;

export type AlignCommand = (typeof ALIGN_COMMANDS)[number];

/** Accessible labels shared by the toolbar and tests. */
export const ALIGN_LABELS: Record<AlignCommand, string> = {
  left: "Align left",
  horizontalCenter: "Align horizontal center",
  right: "Align right",
  top: "Align top",
  verticalCenter: "Align vertical center",
  bottom: "Align bottom",
};

const EPSILON = 1e-6;

/** Objects with a movable rect. Text changes are always locked to their line. */
export function isAlignableObject(object: EditObject): boolean {
  return !object.locked;
}

/**
 * Visual AABB of a PDF rect in displayed page space (origin bottom-left of the
 * page after `/Rotate`). The page box is subtracted before rotating.
 */
export function displayBounds(rect: PdfRect, geometry: PageGeometry): PdfRect {
  const { box, rotate } = geometry;
  const corners = [
    { x: rect.x - box.x, y: rect.y - box.y },
    { x: rect.x - box.x + rect.w, y: rect.y - box.y },
    { x: rect.x - box.x + rect.w, y: rect.y - box.y + rect.h },
    { x: rect.x - box.x, y: rect.y - box.y + rect.h },
  ].map((point) => unrotatedToDisplay(point.x, point.y, box.w, box.h, rotate));
  const xs = corners.map((point) => point.x);
  const ys = corners.map((point) => point.y);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
}

/** A displayed-space delta as an unrotated PDF-space delta. */
export function displayDeltaToPdf(
  dx: number,
  dy: number,
  rotate: PageRotation,
): { x: number; y: number } {
  switch (rotate) {
    case 90:
      return { x: -dy, y: dx };
    case 180:
      return { x: -dx, y: -dy };
    case 270:
      return { x: dy, y: -dx };
    default:
      return { x: dx, y: dy };
  }
}

function unionBounds(rects: PdfRect[]): PdfRect {
  const x = Math.min(...rects.map((rect) => rect.x));
  const y = Math.min(...rects.map((rect) => rect.y));
  const right = Math.max(...rects.map((rect) => rect.x + rect.w));
  const top = Math.max(...rects.map((rect) => rect.y + rect.h));
  return { x, y, w: right - x, h: top - y };
}

/** Keep a translated start coordinate inside [0, limit - size]. */
function clampStart(size: number, limit: number, desired: number): number {
  if (size >= limit) return 0;
  return Math.min(limit - size, Math.max(0, desired));
}

function alignmentDelta(
  command: AlignCommand,
  target: PdfRect,
  bounds: PdfRect,
): { dx: number; dy: number } {
  switch (command) {
    case "left":
      return { dx: target.x - bounds.x, dy: 0 };
    case "horizontalCenter":
      return {
        dx: target.x + target.w / 2 - (bounds.x + bounds.w / 2),
        dy: 0,
      };
    case "right":
      return { dx: target.x + target.w - (bounds.x + bounds.w), dy: 0 };
    case "top":
      return { dx: 0, dy: target.y + target.h - (bounds.y + bounds.h) };
    case "verticalCenter":
      return {
        dx: 0,
        dy: target.y + target.h / 2 - (bounds.y + bounds.h / 2),
      };
    case "bottom":
      return { dx: 0, dy: target.y - bounds.y };
  }
}

export interface AlignSelectionResult {
  objects: EditObject[];
  changedIds: string[];
}

/**
 * Align the movable selected objects on `pageIndex`.
 *
 * Returns `null` when nothing can move or the command changes nothing, so the
 * reducer can leave history untouched.
 */
export function alignSelectedObjects(
  objects: EditObject[],
  selectedIds: string[],
  pageIndex: number,
  geometry: PageGeometry,
  command: AlignCommand,
): AlignSelectionResult | null {
  const selected = new Set(selectedIds);
  const onPage = objects.filter(
    (object) => object.pageIndex === pageIndex && selected.has(object.id),
  );
  const movable = onPage.filter(isAlignableObject);
  if (movable.length === 0) return null;

  const page = displayedSize(geometry);
  if (!(page.w > 0 && page.h > 0)) return null;

  const target: PdfRect =
    onPage.length === 1
      ? { x: 0, y: 0, w: page.w, h: page.h }
      : unionBounds(onPage.map((object) => displayBounds(object.rect, geometry)));

  const changed = new Map<string, EditObject>();
  for (const object of movable) {
    const bounds = displayBounds(object.rect, geometry);
    const delta = alignmentDelta(command, target, bounds);

    let dx = delta.dx;
    let dy = delta.dy;
    if (command === "left" || command === "horizontalCenter" || command === "right") {
      dx = clampStart(bounds.w, page.w, bounds.x + dx) - bounds.x;
    } else {
      dy = clampStart(bounds.h, page.h, bounds.y + dy) - bounds.y;
    }

    const pdf = displayDeltaToPdf(dx, dy, geometry.rotate);
    if (Math.abs(pdf.x) < EPSILON && Math.abs(pdf.y) < EPSILON) continue;
    changed.set(object.id, offsetObject(object, pdf.x, pdf.y));
  }

  if (changed.size === 0) return null;
  return {
    objects: objects.map((object) => changed.get(object.id) ?? object),
    changedIds: [...changed.keys()],
  };
}

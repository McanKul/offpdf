/**
 * Pure helpers for moving an image already painted by the PDF (issue #129).
 *
 * A move is a displacement from the source position, exactly like a source-text move: the image's
 * own bytes and its `/XObject` resource are never touched, so other occurrences of the same image
 * stay where they are. Nothing here is authoritative — every commit is planned by Rust
 * (`preview_text_edits`) and again at Save.
 */

import type { ImageEditIn, TextVec } from "../types";
import { editsSignature, normaliseMove } from "./sourceText";
import type { PdfRect, SourceImageObject, SourceTextObject } from "./types";
import { normalizePdfRect } from "./types";

export interface SourceImageFields {
  locator: string;
  sourceFingerprint: string;
  /** 0-based page in the source file. */
  sourcePageIndex: number;
  move?: TextVec;
}

/** The moved box of an occurrence: its source box shifted by the committed displacement. */
export function movedImageRect(rect: PdfRect, move: TextVec | undefined): PdfRect {
  if (!move) return { ...rect };
  return normalizePdfRect({ ...rect, x: rect.x + move.x, y: rect.y + move.y });
}

/** A locked image-move object (Save reads `id`/`locked` but ignores them). */
export function makeSourceImageObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  fields: SourceImageFields,
): SourceImageObject {
  const move = normaliseMove(fields.move);
  return {
    id,
    kind: "sourceImage",
    pageIndex,
    rect: normalizePdfRect(rect),
    locked: true,
    locator: fields.locator,
    sourceFingerprint: fields.sourceFingerprint,
    sourcePageIndex: fields.sourcePageIndex,
    ...(move ? { move } : {}),
  };
}

/** The IPC shape of one committed image move. */
export function toImageEditIn(obj: SourceImageObject): ImageEditIn {
  return { locator: obj.locator, move: { x: obj.move?.x ?? 0, y: obj.move?.y ?? 0 } };
}

/** Order-independent key of a page's image moves (preview cache key). */
export function imageEditsSignature(images: SourceImageObject[]): string {
  const items = images
    .map((o) => ({ locator: o.locator, move: o.move ?? { x: 0, y: 0 } }))
    .sort((a, b) => (a.locator < b.locator ? -1 : a.locator > b.locator ? 1 : 0));
  return JSON.stringify(items);
}

/** Key of a page's whole change set (text and images), for the preview cache and seeding. */
export function previewSignature(
  texts: SourceTextObject[],
  images: SourceImageObject[],
): string {
  return `${editsSignature(texts)}\u0000${imageEditsSignature(images)}`;
}

/**
 * Edit-document reducer with undo/redo history and gesture coalescing.
 *
 * Selection changes do not create history entries. Structural edits (add /
 * update / delete) push onto the past stack. During a drag (move/resize),
 * BEGIN_GESTURE snapshots once; END_GESTURE finalizes so one gesture = one undo.
 */

import {
  createEmptyDocument,
  lineBounds,
  normalizePdfRect,
  pointsBounds,
  type EditDocument,
  type EditObject,
  type PdfRect,
  type Point,
  type SourceTextObject,
} from "./types";
import { cloneDocument, cloneObject } from "./serialize";
import { isNoOpEdit, normaliseStyle, normaliseTyped } from "./sourceText";
import type { SourceTextStyle, TextRect, TextRun } from "../types";

export const MAX_HISTORY = 100;

export interface HistoryState {
  past: EditDocument[];
  present: EditDocument;
  future: EditDocument[];
  gestureActive: boolean;
  /** True after the first UPDATE inside a gesture (past already snapshotted). */
  gestureCommitted: boolean;
}

export type LayerDir = "front" | "back" | "forward" | "backward";

export type EditAction =
  | { type: "ADD"; object: EditObject }
  | { type: "ADD_MANY"; objects: EditObject[] }
  | { type: "UPDATE"; id: string; patch: Partial<EditObject> }
  | { type: "DELETE"; ids: string[] }
  | { type: "SELECT"; ids: string[] }
  | { type: "CLEAR_SELECTION" }
  | { type: "REORDER"; id: string; dir: LayerDir }
  | { type: "BEGIN_GESTURE" }
  | { type: "END_GESTURE" }
  | { type: "UNDO" }
  | { type: "REDO" }
  | { type: "REPLACE"; document: EditDocument }
  | { type: "REBIND"; present: EditDocument; past: EditDocument[]; future: EditDocument[] }
  | { type: "HYDRATE"; objects: EditObject[] }
  | { type: "RESET" }
  /** Add, or replace text/style/rect of, the sourceText object for (pageIndex, runId). One step. */
  | { type: "UPSERT_SOURCE_TEXT"; object: SourceTextObject }
  /** Delete the sourceText object for (pageIndex, runId), if any. One step. */
  | { type: "REMOVE_SOURCE_TEXT"; pageIndex: number; runId: string }
  /** Delete every sourceText object of one source snapshot (STALE recovery). One step. */
  | { type: "REMOVE_SOURCE_TEXT_FOR_FINGERPRINT"; fingerprint: string };

/** Later objects on the same page paint in front. `null` if the move is a no-op. */
export function reorderOnPage(objects: EditObject[], id: string, dir: LayerDir): EditObject[] | null {
  const from = objects.findIndex((o) => o.id === id);
  if (from < 0) return null;
  const page = objects[from].pageIndex;
  const pagePos = objects.map((o, i) => (o.pageIndex === page ? i : -1)).filter((i) => i >= 0);
  const pos = pagePos.indexOf(from);
  if (pos < 0) return null;
  let target = pos;
  if (dir === "backward") target = pos - 1;
  else if (dir === "forward") target = pos + 1;
  else if (dir === "back") target = 0;
  else target = pagePos.length - 1;
  if (target < 0 || target >= pagePos.length || target === pos) return null;

  const item = objects[from];
  const without = objects.filter((o) => o.id !== id);
  const remain = without
    .map((o, i) => ({ o, i }))
    .filter((x) => x.o.pageIndex === page);
  let insertAt: number;
  if (remain.length === 0) insertAt = without.length;
  else if (target >= remain.length) insertAt = remain[remain.length - 1].i + 1;
  else insertAt = remain[target].i;
  const next = without.slice();
  next.splice(insertAt, 0, item);
  return next;
}

function pushPast(state: HistoryState, nextPresent: EditDocument): HistoryState {
  const past = [...state.past, cloneDocument(state.present)];
  if (past.length > MAX_HISTORY) past.splice(0, past.length - MAX_HISTORY);
  return {
    past,
    present: nextPresent,
    future: [],
    gestureActive: false,
    gestureCommitted: false,
  };
}

export function createHistoryState(doc?: EditDocument): HistoryState {
  return {
    past: [],
    present: doc ? cloneDocument(doc) : createEmptyDocument(),
    future: [],
    gestureActive: false,
    gestureCommitted: false,
  };
}

/** sourceText objects only take `text` and `style` from a patch (redact-guard pattern):
 * geometry, identity, lock and source binding are fixed at creation. */
function sourceTextPatch(o: SourceTextObject, patch: Partial<EditObject>): SourceTextObject {
  const p = patch as Partial<SourceTextObject>;
  return {
    ...o,
    ...(typeof p.text === "string" ? { text: p.text } : {}),
    ...(p.style ? { style: { ...p.style } } : {}),
  };
}

function applyUpdate(
  doc: EditDocument,
  id: string,
  patch: Partial<EditObject>,
): EditDocument {
  return {
    ...doc,
    objects: doc.objects.map((o) => {
      if (o.id !== id) return o;
      if (o.kind === "sourceText") return sourceTextPatch(o, patch);
      const nextPatch = { ...patch };
      if (o.kind === "redact") {
        delete nextPatch.objectRotate;
        delete (nextPatch as { opacity?: number }).opacity;
      }
      const next = { ...o, ...nextPatch } as EditObject;
      if (next.kind === "redact") {
        delete next.objectRotate;
        delete (next as { opacity?: number }).opacity;
      }
      if (patch.rect) {
        next.rect = normalizePdfRect(patch.rect);
        if (
          (next.kind === "highlight" || next.kind === "underline" || next.kind === "strikeout") &&
          !("quads" in patch)
        ) {
          next.quads = quadsFromRect(next.rect);
        }
      }
      return next;
    }),
  };
}

export function editReducer(state: HistoryState, action: EditAction): HistoryState {
  switch (action.type) {
    case "ADD": {
      const present: EditDocument = {
        ...state.present,
        objects: [...state.present.objects, { ...action.object, rect: normalizePdfRect(action.object.rect) }],
        selectedIds: [action.object.id],
      };
      return pushPast({ ...state, gestureActive: false }, present);
    }

    case "ADD_MANY": {
      // Paste / duplicate never copies a text change: one object per (pageIndex, runId).
      const pasted = action.objects.filter((o) => o.kind !== "sourceText");
      if (pasted.length === 0) return state;
      const added = pasted.map((o) => {
        const next = { ...o, rect: normalizePdfRect(o.rect) } as EditObject;
        return next;
      });
      const present: EditDocument = {
        ...state.present,
        objects: [...state.present.objects, ...added],
        selectedIds: added.map((o) => o.id),
      };
      return pushPast({ ...state, gestureActive: false }, present);
    }

    case "UPDATE": {
      if (state.gestureActive) {
        let past = state.past;
        let future = state.future;
        let committed = state.gestureCommitted;
        if (!committed) {
          past = [...state.past, cloneDocument(state.present)];
          if (past.length > MAX_HISTORY) past.splice(0, past.length - MAX_HISTORY);
          future = [];
          committed = true;
        }
        return {
          ...state,
          past,
          future,
          gestureCommitted: committed,
          present: applyUpdate(state.present, action.id, action.patch),
        };
      }
      const present = applyUpdate(state.present, action.id, action.patch);
      return pushPast(state, present);
    }

    case "DELETE": {
      const ids = new Set(action.ids);
      if (ids.size === 0) return state;
      const present: EditDocument = {
        ...state.present,
        objects: state.present.objects.filter((o) => !ids.has(o.id)),
        selectedIds: state.present.selectedIds.filter((id) => !ids.has(id)),
      };
      return pushPast({ ...state, gestureActive: false }, present);
    }

    case "SELECT":
      return {
        ...state,
        present: { ...state.present, selectedIds: [...action.ids] },
      };

    case "CLEAR_SELECTION":
      if (state.present.selectedIds.length === 0) return state;
      return {
        ...state,
        present: { ...state.present, selectedIds: [] },
      };

    case "REORDER": {
      const objects = reorderOnPage(state.present.objects, action.id, action.dir);
      if (!objects) return state;
      return pushPast(state, { ...state.present, objects });
    }

    case "BEGIN_GESTURE": {
      if (state.gestureActive) return state;
      return { ...state, gestureActive: true, gestureCommitted: false };
    }

    case "END_GESTURE":
      return { ...state, gestureActive: false, gestureCommitted: false };

    case "UNDO": {
      if (state.past.length === 0) {
        return { ...state, gestureActive: false, gestureCommitted: false };
      }
      const past = [...state.past];
      const previous = past.pop()!;
      return {
        past,
        present: previous,
        future: [cloneDocument(state.present), ...state.future],
        gestureActive: false,
        gestureCommitted: false,
      };
    }

    case "REDO": {
      if (state.future.length === 0) {
        return { ...state, gestureActive: false, gestureCommitted: false };
      }
      const [next, ...rest] = state.future;
      return {
        past: [...state.past, cloneDocument(state.present)],
        present: next,
        future: rest,
        gestureActive: false,
        gestureCommitted: false,
      };
    }

    case "REPLACE":
      return createHistoryState(action.document);

    case "REBIND":
      return {
        present: cloneDocument(action.present),
        past: action.past.map(cloneDocument),
        future: action.future.map(cloneDocument),
        gestureActive: false,
        gestureCommitted: false,
      };

    case "HYDRATE": {
      if (action.objects.length === 0) return state;
      const added = action.objects.map((o) => {
        const next = { ...o, rect: normalizePdfRect(o.rect) } as EditObject;
        return next;
      });
      const inject = (doc: EditDocument): EditDocument => ({
        ...doc,
        objects: [...doc.objects, ...added.map(cloneObject)],
      });
      return {
        ...state,
        present: inject(state.present),
        past: state.past.map(inject),
        future: state.future.map(inject),
      };
    }

    case "RESET":
      return createHistoryState();

    case "UPSERT_SOURCE_TEXT":
      return upsertSourceText(state, action.object);

    case "REMOVE_SOURCE_TEXT": {
      const ids = state.present.objects
        .filter((o) => o.kind === "sourceText" && o.pageIndex === action.pageIndex && o.runId === action.runId)
        .map((o) => o.id);
      return ids.length === 0 ? state : editReducer(state, { type: "DELETE", ids });
    }

    case "REMOVE_SOURCE_TEXT_FOR_FINGERPRINT": {
      const ids = state.present.objects
        .filter((o) => o.kind === "sourceText" && o.sourceFingerprint === action.fingerprint)
        .map((o) => o.id);
      return ids.length === 0 ? state : editReducer(state, { type: "DELETE", ids });
    }

    default:
      return state;
  }
}

function sameStyle(a: SourceTextStyle, b: SourceTextStyle): boolean {
  return (
    a.sizePt === b.sizePt && a.face === b.face && a.fill === b.fill && a.letterSpacingPt === b.letterSpacingPt
  );
}

function sameRect(a: PdfRect, b: PdfRect): boolean {
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

function upsertSourceText(state: HistoryState, object: SourceTextObject): HistoryState {
  const incoming = { ...object, rect: normalizePdfRect(object.rect), style: { ...object.style } };
  const index = state.present.objects.findIndex(
    (o) => o.kind === "sourceText" && o.pageIndex === incoming.pageIndex && o.runId === incoming.runId,
  );
  if (index < 0) {
    return pushPast(
      { ...state, gestureActive: false },
      { ...state.present, objects: [...state.present.objects, incoming] },
    );
  }
  const existing = state.present.objects[index] as SourceTextObject;
  if (
    existing.text === incoming.text &&
    sameStyle(existing.style, incoming.style) &&
    sameRect(existing.rect, incoming.rect)
  ) {
    return state;
  }
  const replaced: SourceTextObject = { ...existing, text: incoming.text, style: incoming.style, rect: incoming.rect };
  const objects = state.present.objects.map((o, i) => (i === index ? replaced : o));
  return pushPast({ ...state, gestureActive: false }, { ...state.present, objects });
}

export function canUndo(state: HistoryState): boolean {
  return state.past.length > 0;
}

export function canRedo(state: HistoryState): boolean {
  return state.future.length > 0;
}

export interface SourceTextFields {
  runId: string;
  sourceFingerprint: string;
  sourcePageIndex: number;
  originalText: string;
  text: string;
  style: SourceTextStyle;
}

/** A text change object (locked; only `text` and `style` change later). */
export function makeSourceTextObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  fields: SourceTextFields,
): SourceTextObject {
  return {
    id,
    kind: "sourceText",
    pageIndex,
    rect: normalizePdfRect(rect),
    locked: true,
    runId: fields.runId,
    sourceFingerprint: fields.sourceFingerprint,
    sourcePageIndex: fields.sourcePageIndex,
    originalText: fields.originalText,
    text: fields.text,
    style: { ...fields.style },
  };
}

export interface SetSourceTextInput {
  /** Combined (editor) page index. */
  pageIndex: number;
  run: TextRun;
  sourceFingerprint: string;
  /** 0-based page in the source file. */
  sourcePageIndex: number;
  text: string;
  style: SourceTextStyle;
  /** The committed verdict's `newRect`; the run's own box when absent. */
  rect?: TextRect | null;
}

/**
 * The single reducer action for a commit on a run: NFC text and a style
 * reduced to real changes; a no-op removes any existing change (B7), anything
 * else upserts by (pageIndex, runId). `id` is used only when a new object is added.
 */
export function setSourceTextAction(input: SetSourceTextInput, id: string): EditAction {
  const text = normaliseTyped(input.text);
  const style = normaliseStyle(input.run, input.style);
  if (isNoOpEdit(input.run, text, style)) {
    return { type: "REMOVE_SOURCE_TEXT", pageIndex: input.pageIndex, runId: input.run.id };
  }
  return {
    type: "UPSERT_SOURCE_TEXT",
    object: makeSourceTextObject(id, input.pageIndex, input.rect ?? input.run.rect, {
      runId: input.run.id,
      sourceFingerprint: input.sourceFingerprint,
      sourcePageIndex: input.sourcePageIndex,
      originalText: input.run.text,
      text,
      style,
    }),
  };
}

/** Helper to build a draft redaction object (black fill, no label). */
export function makeRedactObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
): import("./types").RedactObject {
  return {
    id,
    kind: "redact",
    pageIndex,
    rect: normalizePdfRect(rect),
    fill: "#000000",
  };
}

/** Helper to build a draft rect object. */
export function makeRectObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  style?: Pick<import("./types").RectObject, "fill" | "stroke" | "strokeWidth" | "opacity">,
): import("./types").RectObject {
  return {
    id,
    kind: "rect",
    pageIndex,
    rect: normalizePdfRect(rect),
    fill: style?.fill ?? "none",
    stroke: style?.stroke ?? "#111827",
    strokeWidth: style?.strokeWidth ?? 1.5,
    opacity: style?.opacity ?? 1,
  };
}

export function makeTextObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  content = "Text",
): import("./types").TextObject {
  return {
    id,
    kind: "text",
    pageIndex,
    rect: normalizePdfRect(rect),
    content,
    fontSize: 14,
    color: "#111827",
    align: "left",
    opacity: 1,
  };
}

export function makeImageObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  path: string,
  previewUrl?: string,
): import("./types").ImageObject {
  return {
    id,
    kind: "image",
    pageIndex,
    rect: normalizePdfRect(rect),
    path,
    keepAspect: true,
    opacity: 1,
    previewUrl,
  };
}

export function makeClosedShape(
  id: string,
  kind: import("./types").ClosedShapeKind,
  pageIndex: number,
  rect: PdfRect,
  style?: import("./types").ShapeStyle,
  keepAspect?: boolean,
): import("./types").EditObject {
  return {
    id,
    kind,
    pageIndex,
    rect: normalizePdfRect(rect),
    fill: style?.fill ?? "none",
    stroke: style?.stroke ?? "#111827",
    strokeWidth: style?.strokeWidth ?? 1.5,
    opacity: style?.opacity ?? 1,
    keepAspect: keepAspect || undefined,
  };
}

export function makeLineObject(
  id: string,
  pageIndex: number,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
): import("./types").LineObject {
  return {
    id,
    kind: "line",
    pageIndex,
    rect: lineBounds(x1, y1, x2, y2),
    x1,
    y1,
    x2,
    y2,
    stroke: "#111827",
    strokeWidth: 2,
    opacity: 1,
  };
}

export function makeInkObject(
  id: string,
  pageIndex: number,
  points: Point[],
): import("./types").InkObject {
  return {
    id,
    kind: "ink",
    pageIndex,
    rect: pointsBounds(points),
    points: points.map((p) => ({ ...p })),
    stroke: "#111827",
    strokeWidth: 2.5,
    opacity: 1,
  };
}

export function makeLinkObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  action: import("./types").LinkAction,
): import("./types").LinkObject {
  return {
    id,
    kind: "link",
    pageIndex,
    rect: normalizePdfRect(rect),
    action,
  };
}

function quadsFromRect(rect: PdfRect): number[] {
  const r = normalizePdfRect(rect);
  return [r.x, r.y, r.x + r.w, r.y, r.x + r.w, r.y + r.h, r.x, r.y + r.h];
}

export function makeNoteObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  author: string,
  color = "#f59e0b",
  comment?: string,
): import("./types").NoteObject {
  return {
    id,
    kind: "note",
    pageIndex,
    rect: normalizePdfRect(rect),
    author,
    color,
    comment,
  };
}

export function makeHighlightObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  author: string,
  color = "#facc15",
  comment?: string,
): import("./types").HighlightObject {
  const box = normalizePdfRect(rect);
  return {
    id,
    kind: "highlight",
    pageIndex,
    rect: box,
    author,
    color,
    comment,
    quads: quadsFromRect(box),
  };
}

export function makeUnderlineObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  author: string,
  color = "#2563eb",
  comment?: string,
): import("./types").UnderlineObject {
  const box = normalizePdfRect(rect);
  return {
    id,
    kind: "underline",
    pageIndex,
    rect: box,
    author,
    color,
    comment,
    quads: quadsFromRect(box),
  };
}

export function makeStrikeoutObject(
  id: string,
  pageIndex: number,
  rect: PdfRect,
  author: string,
  color = "#dc2626",
  comment?: string,
): import("./types").StrikeoutObject {
  const box = normalizePdfRect(rect);
  return {
    id,
    kind: "strikeout",
    pageIndex,
    rect: box,
    author,
    color,
    comment,
    quads: quadsFromRect(box),
  };
}

export function makeMarkupInkObject(
  id: string,
  pageIndex: number,
  strokes: Point[][],
  author: string,
  color = "#111827",
  comment?: string,
): import("./types").MarkupInkObject {
  const points = strokes.flat();
  return {
    id,
    kind: "markupInk",
    pageIndex,
    rect: pointsBounds(points),
    strokes: strokes.map((s) => s.map((p) => ({ ...p }))),
    author,
    color,
    comment,
  };
}

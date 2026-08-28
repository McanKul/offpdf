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
} from "./types";
import { cloneDocument, cloneObject } from "./serialize";

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
  | { type: "RESET" };

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

function applyUpdate(
  doc: EditDocument,
  id: string,
  patch: Partial<EditObject>,
): EditDocument {
  return {
    ...doc,
    objects: doc.objects.map((o) => {
      if (o.id !== id) return o;
      const next = { ...o, ...patch } as EditObject;
      if (patch.rect) {
        next.rect = normalizePdfRect(patch.rect);
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
      if (action.objects.length === 0) return state;
      const added = action.objects.map((o) => {
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

    default:
      return state;
  }
}

export function canUndo(state: HistoryState): boolean {
  return state.past.length > 0;
}

export function canRedo(state: HistoryState): boolean {
  return state.future.length > 0;
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

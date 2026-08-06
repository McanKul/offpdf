/**
 * Edit-document reducer with undo/redo history and gesture coalescing.
 *
 * Selection changes do not create history entries. Structural edits (add /
 * update / delete) push onto the past stack. During a drag (move/resize),
 * BEGIN_GESTURE snapshots once; END_GESTURE finalizes so one gesture = one undo.
 */

import {
  createEmptyDocument,
  normalizePdfRect,
  type EditDocument,
  type EditObject,
  type PdfRect,
} from "./types";

export const MAX_HISTORY = 100;

export interface HistoryState {
  past: EditDocument[];
  present: EditDocument;
  future: EditDocument[];
  /** When set, we are inside a move/resize gesture; past already has the pre-gesture snapshot. */
  gestureActive: boolean;
}

export type EditAction =
  | { type: "ADD"; object: EditObject }
  | { type: "UPDATE"; id: string; patch: Partial<EditObject> }
  | { type: "DELETE"; ids: string[] }
  | { type: "SELECT"; ids: string[] }
  | { type: "CLEAR_SELECTION" }
  | { type: "BEGIN_GESTURE" }
  | { type: "END_GESTURE" }
  | { type: "UNDO" }
  | { type: "REDO" }
  | { type: "REPLACE"; document: EditDocument }
  | { type: "RESET" };

function cloneDoc(doc: EditDocument): EditDocument {
  return {
    version: 1,
    objects: doc.objects.map((o) => ({ ...o, rect: { ...o.rect } })),
    selectedIds: [...doc.selectedIds],
  };
}

function pushPast(state: HistoryState, nextPresent: EditDocument): HistoryState {
  const past = [...state.past, cloneDoc(state.present)];
  if (past.length > MAX_HISTORY) past.splice(0, past.length - MAX_HISTORY);
  return {
    past,
    present: nextPresent,
    future: [],
    gestureActive: state.gestureActive,
  };
}

export function createHistoryState(doc?: EditDocument): HistoryState {
  return {
    past: [],
    present: doc ? cloneDoc(doc) : createEmptyDocument(),
    future: [],
    gestureActive: false,
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

    case "UPDATE": {
      if (state.gestureActive) {
        // Already snapshotted at BEGIN_GESTURE — mutate present only.
        return {
          ...state,
          present: applyUpdate(state.present, action.id, action.patch),
          future: [],
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

    case "BEGIN_GESTURE": {
      if (state.gestureActive) return state;
      // Snapshot current present once; subsequent UPDATE during gesture skip past.
      const past = [...state.past, cloneDoc(state.present)];
      if (past.length > MAX_HISTORY) past.splice(0, past.length - MAX_HISTORY);
      return {
        past,
        present: state.present,
        future: [],
        gestureActive: true,
      };
    }

    case "END_GESTURE":
      return { ...state, gestureActive: false };

    case "UNDO": {
      if (state.past.length === 0) return { ...state, gestureActive: false };
      const past = [...state.past];
      const previous = past.pop()!;
      return {
        past,
        present: previous,
        future: [cloneDoc(state.present), ...state.future],
        gestureActive: false,
      };
    }

    case "REDO": {
      if (state.future.length === 0) return { ...state, gestureActive: false };
      const [next, ...rest] = state.future;
      return {
        past: [...state.past, cloneDoc(state.present)],
        present: next,
        future: rest,
        gestureActive: false,
      };
    }

    case "REPLACE":
      return createHistoryState(action.document);

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
  style?: Pick<import("./types").RectObject, "fill" | "stroke" | "opacity">,
): import("./types").RectObject {
  return {
    id,
    kind: "rect",
    pageIndex,
    rect: normalizePdfRect(rect),
    fill: style?.fill ?? "rgba(37, 99, 235, 0.25)",
    stroke: style?.stroke ?? "#2563eb",
    opacity: style?.opacity ?? 1,
  };
}

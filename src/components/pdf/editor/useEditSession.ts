import { useCallback, useEffect, useLayoutEffect, useReducer, useRef } from "react";
import {
  canRedo,
  canUndo,
  createHistoryState,
  editReducer,
  makeClosedShape,
  makeImageObject,
  makeInkObject,
  makeLineObject,
  makeRectObject,
  makeTextObject,
  mapPointsToRect,
  planKeyRebind,
  type ClosedShapeKind,
  type EditDocument,
  type EditObject,
  type ShapeStyle,
  type LayerDir,
  type PdfRect,
  type Point,
} from "@/lib/editor";

function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `obj-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

export function useEditSession(
  pageKeys: string[] = [],
  onChange?: (doc: EditDocument) => void,
) {
  const [state, dispatch] = useReducer(editReducer, undefined, () => createHistoryState());
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const stateRef = useRef(state);
  stateRef.current = state;
  const prevKeysRef = useRef<string[]>(pageKeys.slice());
  const pageKeysRef = useRef(pageKeys);
  pageKeysRef.current = pageKeys;
  const keysSig = pageKeys.join("\0");

  useEffect(() => {
    onChangeRef.current?.(state.present);
  }, [state.present]);

  useLayoutEffect(() => {
    const nextKeys = pageKeysRef.current.slice();
    const oldKeys = prevKeysRef.current;
    const plan = planKeyRebind(
      stateRef.current.present,
      stateRef.current.past,
      stateRef.current.future,
      oldKeys,
      nextKeys,
    );
    prevKeysRef.current = nextKeys;
    if (!plan) return;
    dispatch({ type: "REBIND", present: plan.present, past: plan.past, future: plan.future });
    onChangeRef.current?.(plan.present);
  }, [keysSig]);

  const addRect = useCallback((pageIndex: number, rect: PdfRect) => {
    dispatch({ type: "ADD", object: makeRectObject(newId(), pageIndex, rect) });
  }, []);

  const addShape = useCallback(
    (kind: ClosedShapeKind, pageIndex: number, rect: PdfRect, style?: ShapeStyle, keepAspect?: boolean) => {
      dispatch({ type: "ADD", object: makeClosedShape(newId(), kind, pageIndex, rect, style, keepAspect) });
    },
    [],
  );

  const addText = useCallback((pageIndex: number, rect: PdfRect, content?: string) => {
    dispatch({ type: "ADD", object: makeTextObject(newId(), pageIndex, rect, content) });
  }, []);

  const addImage = useCallback(
    (pageIndex: number, rect: PdfRect, path: string, previewUrl?: string) => {
      dispatch({ type: "ADD", object: makeImageObject(newId(), pageIndex, rect, path, previewUrl) });
    },
    [],
  );

  const addLine = useCallback((pageIndex: number, x1: number, y1: number, x2: number, y2: number) => {
    dispatch({ type: "ADD", object: makeLineObject(newId(), pageIndex, x1, y1, x2, y2) });
  }, []);

  const addInk = useCallback((pageIndex: number, points: Point[]) => {
    if (points.length < 2) return;
    dispatch({ type: "ADD", object: makeInkObject(newId(), pageIndex, points) });
  }, []);

  const addMany = useCallback((objects: EditObject[]) => {
    if (objects.length === 0) return;
    dispatch({ type: "ADD_MANY", objects });
  }, []);

  const updateObject = useCallback((id: string, patch: Partial<EditObject>) => {
    dispatch({ type: "UPDATE", id, patch });
  }, []);

  const updateRect = useCallback(
    (id: string, rect: PdfRect) => {
      const obj = state.present.objects.find((o) => o.id === id);
      if (!obj) {
        dispatch({ type: "UPDATE", id, patch: { rect } });
        return;
      }
      if (obj.kind === "line") {
        const pts = mapPointsToRect(
          [
            { x: obj.x1, y: obj.y1 },
            { x: obj.x2, y: obj.y2 },
          ],
          obj.rect,
          rect,
        );
        dispatch({
          type: "UPDATE",
          id,
          patch: { rect, x1: pts[0].x, y1: pts[0].y, x2: pts[1].x, y2: pts[1].y } as Partial<EditObject>,
        });
        return;
      }
      if (obj.kind === "ink") {
        dispatch({
          type: "UPDATE",
          id,
          patch: { rect, points: mapPointsToRect(obj.points, obj.rect, rect) } as Partial<EditObject>,
        });
        return;
      }
      dispatch({ type: "UPDATE", id, patch: { rect } });
    },
    [state.present.objects],
  );

  const remove = useCallback((ids: string[]) => {
    dispatch({ type: "DELETE", ids });
  }, []);

  const select = useCallback((ids: string[]) => {
    dispatch({ type: "SELECT", ids });
  }, []);

  const clearSelection = useCallback(() => {
    dispatch({ type: "CLEAR_SELECTION" });
  }, []);

  const beginGesture = useCallback(() => {
    dispatch({ type: "BEGIN_GESTURE" });
  }, []);

  const endGesture = useCallback(() => {
    dispatch({ type: "END_GESTURE" });
  }, []);

  const undo = useCallback(() => {
    dispatch({ type: "UNDO" });
  }, []);

  const redo = useCallback(() => {
    dispatch({ type: "REDO" });
  }, []);

  const reset = useCallback(() => {
    dispatch({ type: "RESET" });
  }, []);

  const reorder = useCallback((id: string, dir: LayerDir) => {
    dispatch({ type: "REORDER", id, dir });
  }, []);

  const nudgeSelected = useCallback(
    (dx: number, dy: number, activePageIndex: number) => {
      const ids = state.present.selectedIds;
      if (ids.length === 0) return;
      dispatch({ type: "BEGIN_GESTURE" });
      for (const id of ids) {
        const obj = state.present.objects.find((o) => o.id === id);
        if (!obj || obj.locked || obj.pageIndex !== activePageIndex) continue;
        const rect = { ...obj.rect, x: obj.rect.x + dx, y: obj.rect.y + dy };
        if (obj.kind === "line") {
          dispatch({
            type: "UPDATE",
            id,
            patch: {
              rect,
              x1: obj.x1 + dx,
              y1: obj.y1 + dy,
              x2: obj.x2 + dx,
              y2: obj.y2 + dy,
            } as Partial<EditObject>,
          });
        } else if (obj.kind === "ink") {
          dispatch({
            type: "UPDATE",
            id,
            patch: {
              rect,
              points: obj.points.map((p) => ({ x: p.x + dx, y: p.y + dy })),
            } as Partial<EditObject>,
          });
        } else {
          dispatch({ type: "UPDATE", id, patch: { rect } });
        }
      }
      dispatch({ type: "END_GESTURE" });
    },
    [state.present.objects, state.present.selectedIds],
  );

  return {
    document: state.present as EditDocument,
    objects: state.present.objects as EditObject[],
    selectedIds: state.present.selectedIds,
    past: state.past as EditDocument[],
    future: state.future as EditDocument[],
    canUndo: canUndo(state),
    canRedo: canRedo(state),
    addRect,
    addShape,
    addText,
    addImage,
    addLine,
    addInk,
    addMany,
    updateRect,
    updateObject,
    remove,
    select,
    clearSelection,
    beginGesture,
    endGesture,
    undo,
    redo,
    reset,
    reorder,
    nudgeSelected,
  };
}

export type EditSession = ReturnType<typeof useEditSession>;

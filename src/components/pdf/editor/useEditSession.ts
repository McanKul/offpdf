import { useCallback, useEffect, useReducer, useRef } from "react";
import {
  canRedo,
  canUndo,
  createHistoryState,
  editReducer,
  makeRectObject,
  type EditDocument,
  type EditObject,
  type PdfRect,
} from "@/lib/editor";

function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `obj-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

export function useEditSession(onChange?: (doc: EditDocument) => void) {
  const [state, dispatch] = useReducer(editReducer, undefined, () => createHistoryState());
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    onChangeRef.current?.(state.present);
  }, [state.present]);

  const addRect = useCallback((pageIndex: number, rect: PdfRect) => {
    dispatch({
      type: "ADD",
      object: makeRectObject(newId(), pageIndex, rect),
    });
  }, []);

  const updateRect = useCallback((id: string, rect: PdfRect) => {
    dispatch({ type: "UPDATE", id, patch: { rect } });
  }, []);

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

  const nudgeSelected = useCallback(
    (dx: number, dy: number) => {
      const ids = state.present.selectedIds;
      if (ids.length === 0) return;
      // One history step for multi-nudge: begin gesture style via sequential updates
      // For simplicity, update each selected object with its own history entry is noisy;
      // nudge applies as discrete UPDATE steps (keyboard).
      for (const id of ids) {
        const obj = state.present.objects.find((o) => o.id === id);
        if (!obj || obj.locked) continue;
        dispatch({
          type: "UPDATE",
          id,
          patch: {
            rect: {
              ...obj.rect,
              x: obj.rect.x + dx,
              y: obj.rect.y + dy,
            },
          },
        });
      }
    },
    [state.present.objects, state.present.selectedIds],
  );

  /** Keyboard bindings when the editor region is focused / active. */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      const tag = t?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || t?.isContentEditable) return;

      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "z" && !e.shiftKey) {
        e.preventDefault();
        dispatch({ type: "UNDO" });
        return;
      }
      if (mod && (e.key.toLowerCase() === "y" || (e.key.toLowerCase() === "z" && e.shiftKey))) {
        e.preventDefault();
        dispatch({ type: "REDO" });
        return;
      }
      if (e.key === "Escape") {
        dispatch({ type: "CLEAR_SELECTION" });
        return;
      }
      if (e.key === "Delete" || e.key === "Backspace") {
        if (state.present.selectedIds.length === 0) return;
        e.preventDefault();
        dispatch({ type: "DELETE", ids: state.present.selectedIds });
        return;
      }
      const step = e.shiftKey ? 10 : 1;
      if (e.key === "ArrowLeft") {
        e.preventDefault();
        nudgeSelected(-step, 0);
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        nudgeSelected(step, 0);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        // PDF y increases upward
        nudgeSelected(0, step);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        nudgeSelected(0, -step);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [nudgeSelected, state.present.selectedIds]);

  return {
    document: state.present as EditDocument,
    objects: state.present.objects as EditObject[],
    selectedIds: state.present.selectedIds,
    canUndo: canUndo(state),
    canRedo: canRedo(state),
    addRect,
    updateRect,
    remove,
    select,
    clearSelection,
    beginGesture,
    endGesture,
    undo,
    redo,
  };
}

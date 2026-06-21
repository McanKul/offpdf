/**
 * Pointer-based drag-to-reorder. We deliberately avoid the HTML5 drag-and-drop
 * API because Tauri's native window drag-drop (used for dropping files INTO the
 * app) intercepts HTML5 drags on macOS/WKWebView — the drag turns into a window
 * "file drop" instead. Pointer events don't start a native drag, so in-app
 * sorting works alongside file drop.
 *
 * Tiles/rows must carry a `data-sort-idx={index}` attribute and call
 * `begin(index)` from `onPointerDown`.
 */
import { useEffect, useRef, useState } from "react";

export function useSortable(onReorder: (from: number, to: number) => void) {
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [overIndex, setOverIndex] = useState<number | null>(null);
  const dragRef = useRef<number | null>(null);
  const overRef = useRef<number | null>(null);
  const cbRef = useRef(onReorder);
  cbRef.current = onReorder;

  const begin = (index: number) => (e: React.PointerEvent) => {
    // Left button / touch / pen only.
    if (e.button !== 0 && e.pointerType === "mouse") return;
    e.preventDefault();
    dragRef.current = index;
    overRef.current = index;
    setDragIndex(index);
    setOverIndex(index);
  };

  useEffect(() => {
    if (dragIndex === null) return;

    const resolveIdx = (x: number, y: number): number | null => {
      const el = document.elementFromPoint(x, y) as HTMLElement | null;
      const tile = el?.closest("[data-sort-idx]") as HTMLElement | null;
      if (!tile) return null;
      const idx = Number(tile.dataset.sortIdx);
      return Number.isNaN(idx) ? null : idx;
    };

    const onMove = (e: PointerEvent) => {
      const idx = resolveIdx(e.clientX, e.clientY);
      if (idx !== null && idx !== overRef.current) {
        overRef.current = idx;
        setOverIndex(idx);
      }
    };
    const onUp = () => {
      const from = dragRef.current;
      const to = overRef.current;
      dragRef.current = null;
      overRef.current = null;
      setDragIndex(null);
      setOverIndex(null);
      if (from !== null && to !== null && from !== to) cbRef.current(from, to);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  }, [dragIndex]);

  return { dragIndex, overIndex, begin };
}

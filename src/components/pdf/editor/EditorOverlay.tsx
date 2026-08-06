/**
 * SVG overlay for draft objects: select, move, resize. Coordinates go through
 * the pure editor mapping so PDF export coords stay stable under zoom/rotation.
 */
import { useRef, useState } from "react";
import {
  makeMapping,
  pdfRectToViewport,
  resizePdfRect,
  viewportToPdf,
  type EditObject,
  type PdfRect,
  type ResizeHandle,
  type ViewportMapping,
} from "@/lib/editor";
import type { PageLayout } from "./PageSurface";

type Handle = ResizeHandle;

type DragMode =
  | { kind: "move"; id: string; startPdf: PdfRect; startCss: { x: number; y: number } }
  | {
      kind: "resize";
      id: string;
      handle: Handle;
      startPdf: PdfRect;
      startCss: { x: number; y: number };
    }
  | { kind: "create"; startCss: { x: number; y: number } };

function clientToLocal(
  e: { clientX: number; clientY: number },
  svg: SVGSVGElement,
): { x: number; y: number } {
  const rect = svg.getBoundingClientRect();
  return { x: e.clientX - rect.left, y: e.clientY - rect.top };
}

function cssRectToPdf(
  css: { x: number; y: number; w: number; h: number },
  mapping: ViewportMapping,
): PdfRect {
  const corners = [
    { x: css.x, y: css.y },
    { x: css.x + css.w, y: css.y },
    { x: css.x + css.w, y: css.y + css.h },
    { x: css.x, y: css.y + css.h },
  ].map((p) => viewportToPdf(p, mapping));
  const xs = corners.map((p) => p.x);
  const ys = corners.map((p) => p.y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
}

export function EditorOverlay({
  layout,
  objects,
  selectedIds,
  pageIndex,
  createMode,
  onSelect,
  onClearSelection,
  onBeginGesture,
  onEndGesture,
  onUpdateRect,
  onCreateRect,
}: {
  layout: PageLayout;
  objects: EditObject[];
  selectedIds: string[];
  pageIndex: number;
  createMode: boolean;
  onSelect: (ids: string[]) => void;
  onClearSelection: () => void;
  onBeginGesture: () => void;
  onEndGesture: () => void;
  onUpdateRect: (id: string, rect: PdfRect) => void;
  onCreateRect: (rect: PdfRect) => void;
}) {
  const svgRef = useRef<SVGSVGElement>(null);
  const dragRef = useRef<DragMode | null>(null);
  const [draftCreate, setDraftCreate] = useState<{
    start: { x: number; y: number };
    cur: { x: number; y: number };
  } | null>(null);

  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);
  const pageObjects = objects.filter((o) => o.pageIndex === pageIndex);

  const onPointerDownBg = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const svg = svgRef.current;
    if (!svg) return;
    const local = clientToLocal(e, svg);
    if (createMode) {
      dragRef.current = { kind: "create", startCss: local };
      setDraftCreate({ start: local, cur: local });
      svg.setPointerCapture(e.pointerId);
      e.preventDefault();
      return;
    }
    onClearSelection();
  };

  const onPointerDownObject = (e: React.PointerEvent, obj: EditObject) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const svg = svgRef.current;
    if (!svg) return;
    onSelect([obj.id]);
    if (obj.locked) return;
    const local = clientToLocal(e, svg);
    onBeginGesture();
    dragRef.current = {
      kind: "move",
      id: obj.id,
      startPdf: { ...obj.rect },
      startCss: local,
    };
    svg.setPointerCapture(e.pointerId);
  };

  const onPointerDownHandle = (
    e: React.PointerEvent,
    obj: EditObject,
    handle: Handle,
  ) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const svg = svgRef.current;
    if (!svg) return;
    onSelect([obj.id]);
    const local = clientToLocal(e, svg);
    onBeginGesture();
    dragRef.current = {
      kind: "resize",
      id: obj.id,
      handle,
      startPdf: { ...obj.rect },
      startCss: local,
    };
    svg.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const drag = dragRef.current;
    const svg = svgRef.current;
    if (!drag || !svg) return;
    const local = clientToLocal(e, svg);

    if (drag.kind === "create") {
      setDraftCreate({ start: drag.startCss, cur: local });
      return;
    }

    const startPt = viewportToPdf(drag.startCss, mapping);
    const curPt = viewportToPdf(local, mapping);
    const dPdf = { x: curPt.x - startPt.x, y: curPt.y - startPt.y };

    if (drag.kind === "move") {
      onUpdateRect(drag.id, {
        ...drag.startPdf,
        x: drag.startPdf.x + dPdf.x,
        y: drag.startPdf.y + dPdf.y,
      });
    } else {
      onUpdateRect(drag.id, resizePdfRect(drag.startPdf, drag.handle, dPdf.x, dPdf.y));
    }
  };

  const onPointerUp = (e: React.PointerEvent) => {
    const drag = dragRef.current;
    const svg = svgRef.current;
    if (!drag || !svg) return;
    try {
      svg.releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }

    if (drag.kind === "create") {
      const local = clientToLocal(e, svg);
      const x = Math.min(drag.startCss.x, local.x);
      const y = Math.min(drag.startCss.y, local.y);
      const w = Math.abs(local.x - drag.startCss.x);
      const h = Math.abs(local.y - drag.startCss.y);
      setDraftCreate(null);
      dragRef.current = null;
      if (w >= 4 && h >= 4) {
        onCreateRect(cssRectToPdf({ x, y, w, h }, mapping));
      }
      return;
    }

    dragRef.current = null;
    onEndGesture();
  };

  return (
    <svg
      ref={svgRef}
      className="pdf-editor__overlay"
      width={layout.cssWidth}
      height={layout.cssHeight}
      role="group"
      aria-label="Page editor overlay"
      onPointerDown={onPointerDownBg}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <rect x={0} y={0} width={layout.cssWidth} height={layout.cssHeight} fill="transparent" />

      {pageObjects.map((obj) => {
        const css = pdfRectToViewport(obj.rect, mapping);
        const selected = selectedIds.includes(obj.id);
        const fill =
          obj.kind === "rect" ? (obj.fill ?? "rgba(37,99,235,0.25)") : "rgba(37,99,235,0.25)";
        const stroke =
          selected
            ? "#2563eb"
            : obj.kind === "rect"
              ? (obj.stroke ?? "#2563eb")
              : "#2563eb";
        return (
          <g key={obj.id}>
            <rect
              x={css.x}
              y={css.y}
              width={Math.max(css.w, 1)}
              height={Math.max(css.h, 1)}
              fill={fill}
              stroke={stroke}
              strokeWidth={selected ? 2 : 1.5}
              opacity={obj.kind === "rect" ? (obj.opacity ?? 1) : 1}
              style={{ cursor: obj.locked ? "default" : "move" }}
              onPointerDown={(e) => onPointerDownObject(e, obj)}
            />
            {selected &&
              !obj.locked &&
              (["nw", "ne", "sw", "se"] as Handle[]).map((handle) => {
                const hx = handle === "nw" || handle === "sw" ? css.x : css.x + css.w;
                const hy = handle === "nw" || handle === "ne" ? css.y : css.y + css.h;
                return (
                  <rect
                    key={handle}
                    x={hx - 5}
                    y={hy - 5}
                    width={10}
                    height={10}
                    fill="#fff"
                    stroke="#2563eb"
                    strokeWidth={1.5}
                    style={{
                      cursor:
                        handle === "nw" || handle === "se" ? "nwse-resize" : "nesw-resize",
                    }}
                    onPointerDown={(e) => onPointerDownHandle(e, obj, handle)}
                  />
                );
              })}
          </g>
        );
      })}

      {draftCreate && (
        <rect
          x={Math.min(draftCreate.start.x, draftCreate.cur.x)}
          y={Math.min(draftCreate.start.y, draftCreate.cur.y)}
          width={Math.abs(draftCreate.cur.x - draftCreate.start.x)}
          height={Math.abs(draftCreate.cur.y - draftCreate.start.y)}
          fill="rgba(37,99,235,0.15)"
          stroke="#2563eb"
          strokeDasharray="4 3"
          pointerEvents="none"
        />
      )}
    </svg>
  );
}

/**
 * SVG overlay: select, move, resize, and create text/rect/line/ink/image drafts.
 * Resize happens in CSS space then maps back so /Rotate pages stay correct.
 */
import { useRef, useState } from "react";
import {
  bubbleSvgPath,
  closedShapeCssPoints,
  cssCenter,
  displayedSize,
  isClosedShapeObject,
  isNoneFill,
  makeMapping,
  moveSelectedRects,
  normalizeDeg,
  pdfRectToViewport,
  pdfToViewport,
  pointerAngleDeg,
  aspectLocked,
  constrainCssBox1to1,
  cssBoxFromPoints,
  resizeCssRect,
  resizeCssRectLocked,
  rotateCss,
  snapDeg,
  viewportToPdf,
  type ClosedShapeKind,
  type EditObject,
  type PdfRect,
  type Point,
  type ResizeHandle,
  type ShapeStyle,
  type ViewportMapping,
} from "@/lib/editor";
import type { PageLayout } from "./PageSurface";
import { shapeKindForTool, toolForces1to1 } from "./ShapePicker";

export type EditorTool =
  | "select"
  | "hand"
  | "text"
  | "rect"
  | "square"
  | "roundRect"
  | "ellipse"
  | "circle"
  | "triangle"
  | "star"
  | "hexagon"
  | "bubble"
  | "arrow"
  | "line"
  | "ink"
  | "image"
  | "link"
  | "note"
  | "highlight"
  | "underline"
  | "strikeout"
  | "markupInk"
  | "redact";

type Handle = ResizeHandle;

type DragMode =
  | {
      kind: "move";
      ids: string[];
      starts: Record<string, PdfRect>;
      startCss: { x: number; y: number };
    }
  | {
      kind: "resize";
      id: string;
      handle: Handle;
      startCssRect: { x: number; y: number; w: number; h: number };
      startCss: { x: number; y: number };
      center: { x: number; y: number };
      rot: number;
    }
  | {
      kind: "rotate";
      id: string;
      startAngle: number;
      center: { x: number; y: number };
      startPointerAngle: number;
    }
  | { kind: "marquee"; startCss: { x: number; y: number }; additive: boolean }
  | { kind: "create-shape"; shape: ClosedShapeKind; startCss: { x: number; y: number }; lock1to1: boolean }
  | { kind: "create-text"; startCss: { x: number; y: number } }
  | { kind: "create-link"; startCss: { x: number; y: number } }
  | { kind: "create-line"; startCss: { x: number; y: number } }
  | { kind: "create-ink"; points: { x: number; y: number }[] }
  | { kind: "create-markup"; markup: "note" | "highlight" | "underline" | "strikeout"; startCss: { x: number; y: number } }
  | { kind: "create-markup-ink"; points: { x: number; y: number }[] }
  | { kind: "create-redact"; startCss: { x: number; y: number } };

function additiveSelect(e: { shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }) {
  return e.shiftKey || e.metaKey || e.ctrlKey;
}

function cssIntersects(
  a: { x: number; y: number; w: number; h: number },
  b: { x: number; y: number; w: number; h: number },
) {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
}

function clientToLocal(
  e: { clientX: number; clientY: number },
  svg: SVGSVGElement,
): { x: number; y: number } {
  const rect = svg.getBoundingClientRect();
  return { x: e.clientX - rect.left, y: e.clientY - rect.top };
}

export function EditorOverlay({
  layout,
  objects,
  selectedIds,
  pageIndex,
  tool,
  onSelect,
  onClearSelection,
  onBeginGesture,
  onEndGesture,
  onUpdateRect,
  onUpdateRotate,
  onCreateShape,
  onCreateText,
  onCreateLink,
  onCreateLine,
  onCreateInk,
  onCreateNote,
  onCreateHighlight,
  onCreateUnderline,
  onCreateStrikeout,
  onCreateMarkupInk,
  onCreateRedact,
  onRequestImage,
  onActivateText,
  pickColor,
  onPickColor,
  onPickHover,
  createStyle,
}: {
  layout: PageLayout;
  objects: EditObject[];
  selectedIds: string[];
  pageIndex: number;
  tool: EditorTool;
  onSelect: (ids: string[]) => void;
  onClearSelection: () => void;
  onBeginGesture: () => void;
  onEndGesture: () => void;
  onUpdateRect: (id: string, rect: PdfRect) => void;
  onUpdateRotate: (id: string, deg: number) => void;
  onCreateShape: (kind: ClosedShapeKind, rect: PdfRect, keepAspect?: boolean) => void;
  onCreateText: (rect: PdfRect) => void;
  onCreateLink: (rect: PdfRect) => void;
  onCreateLine: (a: Point, b: Point) => void;
  onCreateInk: (points: Point[]) => void;
  onCreateNote: (rect: PdfRect) => void;
  onCreateHighlight: (rect: PdfRect) => void;
  onCreateUnderline: (rect: PdfRect) => void;
  onCreateStrikeout: (rect: PdfRect) => void;
  onCreateMarkupInk: (strokes: Point[][]) => void;
  onCreateRedact: (rect: PdfRect) => void;
  onRequestImage: (atCss: { x: number; y: number }) => void;
  onActivateText: (id: string) => void;
  /** When set, the next click samples a page color instead of editing. */
  pickColor?: boolean;
  onPickColor?: (css: { x: number; y: number }) => void;
  onPickHover?: (pos: { x: number; y: number } | null) => void;
  /** Fill/stroke used while rubber-banding a new shape. */
  createStyle?: ShapeStyle;
}) {
  const svgRef = useRef<SVGSVGElement>(null);
  const dragRef = useRef<DragMode | null>(null);
  const [draft, setDraft] = useState<{
    kind: "box" | "shape" | "line" | "ink";
    start: { x: number; y: number };
    cur: { x: number; y: number };
    points?: { x: number; y: number }[];
    shape?: ClosedShapeKind;
  } | null>(null);
  const style: ShapeStyle = createStyle ?? {};

  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);
  const pageObjects = objects.filter((o) => o.pageIndex === pageIndex);

  const onPointerDownBg = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const svg = svgRef.current;
    if (!svg) return;
    const local = clientToLocal(e, svg);

    if (pickColor) {
      onPickColor?.(local);
      e.preventDefault();
      return;
    }
    if (tool === "hand") return;

    if (tool === "image") {
      onRequestImage(local);
      return;
    }
    if (tool === "select") {
      dragRef.current = { kind: "marquee", startCss: local, additive: additiveSelect(e) };
      setDraft({ kind: "box", start: local, cur: local });
      svg.setPointerCapture(e.pointerId);
      e.preventDefault();
      return;
    }
    const shapeKind = shapeKindForTool(tool);
    if (shapeKind) {
      dragRef.current = {
        kind: "create-shape",
        shape: shapeKind,
        startCss: local,
        lock1to1: toolForces1to1(tool),
      };
      setDraft({ kind: "shape", shape: shapeKind, start: local, cur: local });
    } else if (tool === "text") {
      dragRef.current = { kind: "create-text", startCss: local };
      setDraft({ kind: "box", start: local, cur: local });
    } else if (tool === "link") {
      dragRef.current = { kind: "create-link", startCss: local };
      setDraft({ kind: "box", start: local, cur: local });
    } else if (tool === "line") {
      dragRef.current = { kind: "create-line", startCss: local };
      setDraft({ kind: "line", start: local, cur: local });
    } else if (tool === "ink") {
      dragRef.current = { kind: "create-ink", points: [local] };
      setDraft({ kind: "ink", start: local, cur: local, points: [local] });
    } else if (tool === "note" || tool === "highlight" || tool === "underline" || tool === "strikeout") {
      dragRef.current = { kind: "create-markup", markup: tool, startCss: local };
      setDraft({ kind: "box", start: local, cur: local });
    } else if (tool === "markupInk") {
      dragRef.current = { kind: "create-markup-ink", points: [local] };
      setDraft({ kind: "ink", start: local, cur: local, points: [local] });
    } else if (tool === "redact") {
      dragRef.current = { kind: "create-redact", startCss: local };
      setDraft({ kind: "box", start: local, cur: local });
    }
    svg.setPointerCapture(e.pointerId);
    e.preventDefault();
  };

  const onPointerDownObject = (e: React.PointerEvent, obj: EditObject) => {
    if (e.button !== 0) return;
    const svg = svgRef.current;
    if (!svg) return;
    if (tool === "hand") return;
    // Create tools (square, text, ink, …) must start a new object even if the
    // pointer is over an existing one — don't steal the event from the page.
    if (!pickColor && tool !== "select") return;
    e.stopPropagation();
    if (pickColor) {
      onPickColor?.(clientToLocal(e, svg));
      return;
    }
    if (additiveSelect(e)) {
      const next = selectedIds.includes(obj.id)
        ? selectedIds.filter((id) => id !== obj.id)
        : [...selectedIds, obj.id];
      onSelect(next);
      return;
    }
    const movingIds = selectedIds.includes(obj.id) ? selectedIds : [obj.id];
    if (!selectedIds.includes(obj.id)) onSelect([obj.id]);
    if (tool !== "select") return;
    if (obj.locked) return;
    if (obj.kind === "text" && e.detail >= 2 && movingIds.length === 1) {
      onActivateText(obj.id);
      return;
    }
    const local = clientToLocal(e, svg);
    const starts: Record<string, PdfRect> = {};
    for (const id of movingIds) {
      const o = objects.find((x) => x.id === id);
      if (o) starts[id] = { ...o.rect };
    }
    onBeginGesture();
    dragRef.current = { kind: "move", ids: movingIds, starts, startCss: local };
    svg.setPointerCapture(e.pointerId);
  };

  const onPointerDownHandle = (e: React.PointerEvent, obj: EditObject, handle: Handle) => {
    if (e.button !== 0) return;
    if (tool !== "select") return;
    e.stopPropagation();
    const svg = svgRef.current;
    if (!svg) return;
    onSelect([obj.id]);
    const local = clientToLocal(e, svg);
    const box = pdfRectToViewport(obj.rect, mapping);
    onBeginGesture();
    dragRef.current = {
      kind: "resize",
      id: obj.id,
      handle,
      startCssRect: box,
      startCss: local,
      center: cssCenter(box),
      rot: obj.objectRotate ?? 0,
    };
    svg.setPointerCapture(e.pointerId);
  };

  const onPointerDownRotate = (e: React.PointerEvent, obj: EditObject) => {
    if (e.button !== 0) return;
    if (tool !== "select") return;
    e.stopPropagation();
    const svg = svgRef.current;
    if (!svg) return;
    onSelect([obj.id]);
    const local = clientToLocal(e, svg);
    const box = pdfRectToViewport(obj.rect, mapping);
    const center = cssCenter(box);
    onBeginGesture();
    dragRef.current = {
      kind: "rotate",
      id: obj.id,
      startAngle: obj.objectRotate ?? 0,
      center,
      startPointerAngle: pointerAngleDeg(local, center),
    };
    svg.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (pickColor) {
      onPickHover?.({ x: e.clientX, y: e.clientY });
    }
    const drag = dragRef.current;
    const svg = svgRef.current;
    if (!drag || !svg) return;
    const local = clientToLocal(e, svg);

    if (drag.kind === "create-shape") {
      if (drag.lock1to1 || e.shiftKey) {
        const box = constrainCssBox1to1(drag.startCss, local);
        setDraft({
          kind: "shape",
          shape: drag.shape,
          start: { x: box.x, y: box.y },
          cur: { x: box.x + box.w, y: box.y + box.h },
        });
      } else {
        setDraft({ kind: "shape", shape: drag.shape, start: drag.startCss, cur: local });
      }
      return;
    }
    if (
      drag.kind === "create-text" ||
      drag.kind === "create-link" ||
      drag.kind === "create-markup" ||
      drag.kind === "create-redact" ||
      drag.kind === "marquee"
    ) {
      setDraft({ kind: "box", start: drag.startCss, cur: local });
      return;
    }
    if (drag.kind === "create-line") {
      setDraft({ kind: "line", start: drag.startCss, cur: local });
      return;
    }
    if (drag.kind === "create-ink" || drag.kind === "create-markup-ink") {
      const pts = [...drag.points, local];
      drag.points = pts;
      setDraft({ kind: "ink", start: pts[0], cur: local, points: pts });
      return;
    }

    if (drag.kind === "move") {
      const startPt = viewportToPdf(drag.startCss, mapping);
      const curPt = viewportToPdf(local, mapping);
      const dx = curPt.x - startPt.x;
      const dy = curPt.y - startPt.y;
      const fromStarts = objects.map((o) => {
        const start = drag.starts[o.id];
        return start ? ({ ...o, rect: { ...start } } as EditObject) : o;
      });
      const moved = moveSelectedRects(fromStarts, drag.ids, pageIndex, dx, dy);
      for (const o of moved) {
        const start = drag.starts[o.id];
        if (!start) continue;
        if (o.rect.x === start.x && o.rect.y === start.y) continue;
        onUpdateRect(o.id, o.rect);
      }
      return;
    }

    if (drag.kind === "rotate") {
      const delta = pointerAngleDeg(local, drag.center) - drag.startPointerAngle;
      let deg = drag.startAngle + delta;
      if (e.shiftKey) deg = snapDeg(deg);
      onUpdateRotate(drag.id, normalizeDeg(deg));
      return;
    }

    const from = rotateCss(drag.startCss, drag.center, -drag.rot);
    const to = rotateCss(local, drag.center, -drag.rot);
    const obj = objects.find((o) => o.id === drag.id);
    const lock = obj ? aspectLocked(obj, e.shiftKey) : false;
    const nextCss = lock
      ? resizeCssRectLocked(drag.startCssRect, drag.handle, to.x - from.x, to.y - from.y)
      : resizeCssRect(drag.startCssRect, drag.handle, to.x - from.x, to.y - from.y);
    onUpdateRect(drag.id, viewportRectFromCss(nextCss, mapping));
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
    const local = clientToLocal(e, svg);

    if (drag.kind === "marquee") {
      const box = cssBoxFromPoints(drag.startCss, local);
      setDraft(null);
      dragRef.current = null;
      if (box.w < 3 && box.h < 3) {
        if (!drag.additive) onClearSelection();
        return;
      }
      const hit = pageObjects
        .filter((o) => cssIntersects(box, pdfRectToViewport(o.rect, mapping)))
        .map((o) => o.id);
      if (drag.additive) {
        const set = new Set(selectedIds);
        for (const id of hit) set.add(id);
        onSelect([...set]);
      } else {
        onSelect(hit);
      }
      return;
    }
    if (drag.kind === "create-markup") {
      const box = cssBoxFromPoints(drag.startCss, local);
      setDraft(null);
      dragRef.current = null;
      const click = box.w < 4 || box.h < 4;
      const fallback = click
        ? {
            x: drag.startCss.x,
            y: drag.startCss.y,
            w: drag.markup === "note" ? 28 : 160,
            h: drag.markup === "note" ? 28 : drag.markup === "highlight" ? 18 : 12,
          }
        : box;
      const pdf = viewportRectFromCss(fallback, mapping);
      if (drag.markup === "note") onCreateNote(pdf);
      else if (drag.markup === "highlight") onCreateHighlight(pdf);
      else if (drag.markup === "underline") onCreateUnderline(pdf);
      else onCreateStrikeout(pdf);
      return;
    }
    if (
      drag.kind === "create-shape" ||
      drag.kind === "create-text" ||
      drag.kind === "create-link" ||
      drag.kind === "create-redact"
    ) {
      const lock1to1 = drag.kind === "create-shape" && (drag.lock1to1 || e.shiftKey);
      const box = lock1to1 ? constrainCssBox1to1(drag.startCss, local) : cssBoxFromPoints(drag.startCss, local);
      setDraft(null);
      dragRef.current = null;
      const click = box.w < 4 || box.h < 4;
      const fallback = click
        ? {
            x: drag.startCss.x,
            y: drag.startCss.y,
            w: drag.kind === "create-text" ? 160 : lock1to1 ? 120 : 160,
            h: drag.kind === "create-text" ? 36 : lock1to1 ? 120 : 80,
          }
        : box;
      const pdf = viewportRectFromCss(fallback, mapping);
      if (drag.kind === "create-text") onCreateText(pdf);
      else if (drag.kind === "create-link") onCreateLink(pdf);
      else if (drag.kind === "create-redact") onCreateRedact(pdf);
      else onCreateShape(drag.shape, pdf, drag.lock1to1 || undefined);
      return;
    }
    if (drag.kind === "create-line") {
      setDraft(null);
      dragRef.current = null;
      onCreateLine(viewportToPdf(drag.startCss, mapping), viewportToPdf(local, mapping));
      return;
    }
    if (drag.kind === "create-ink") {
      setDraft(null);
      dragRef.current = null;
      const pts = drag.points.map((p) => viewportToPdf(p, mapping));
      onCreateInk(pts);
      return;
    }
    if (drag.kind === "create-markup-ink") {
      setDraft(null);
      dragRef.current = null;
      const pts = drag.points.map((p) => viewportToPdf(p, mapping));
      onCreateMarkupInk([pts]);
      return;
    }

    dragRef.current = null;
    onEndGesture();
  };

  const cursor = pickColor ? "none" : tool === "hand" ? "grab" : tool === "select" ? "default" : "crosshair";
  const objectsInteractive = tool === "select" || !!pickColor;
  const shapeDraftCss =
    draft?.kind === "shape" && draft.shape ? cssBoxFromPoints(draft.start, draft.cur) : null;

  return (
    <svg
      ref={svgRef}
      className={`pdf-editor__overlay${pickColor ? " is-eyedrop" : ""}${tool === "hand" ? " is-hand" : ""}`}
      width={layout.cssWidth}
      height={layout.cssHeight}
      role="group"
      aria-label="Page editor overlay"
      style={{ cursor }}
      onPointerDown={onPointerDownBg}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onPointerLeave={() => onPickHover?.(null)}
    >
      <defs>
        <pattern
          id="offpdf-redact-hatch"
          width="8"
          height="8"
          patternUnits="userSpaceOnUse"
          patternTransform="rotate(45)"
        >
          <rect width="8" height="8" fill="rgba(185,28,28,0.08)" />
          <line x1="0" y1="0" x2="0" y2="8" stroke="rgba(185,28,28,0.45)" strokeWidth="2.5" />
        </pattern>
      </defs>
      <rect x={0} y={0} width={layout.cssWidth} height={layout.cssHeight} fill="transparent" />

      {pageObjects.map((obj) => (
        <ObjectShape
          key={obj.id}
          obj={obj}
          mapping={mapping}
          selected={selectedIds.includes(obj.id)}
          selectedCount={selectedIds.length}
          interactive={objectsInteractive}
          onPointerDownObject={onPointerDownObject}
          onPointerDownHandle={onPointerDownHandle}
          onPointerDownRotate={onPointerDownRotate}
        />
      ))}

      {draft?.kind === "box" && (
        <rect
          x={Math.min(draft.start.x, draft.cur.x)}
          y={Math.min(draft.start.y, draft.cur.y)}
          width={Math.abs(draft.cur.x - draft.start.x)}
          height={Math.abs(draft.cur.y - draft.start.y)}
          fill={tool === "redact" ? "url(#offpdf-redact-hatch)" : "rgba(37,99,235,0.12)"}
          stroke={tool === "redact" ? "#b91c1c" : "#2563eb"}
          strokeDasharray={tool === "redact" ? undefined : "4 3"}
          strokeWidth={tool === "redact" ? 2 : 1}
          pointerEvents="none"
        />
      )}
      {draft?.kind === "shape" && draft.shape && shapeDraftCss && (shapeDraftCss.w >= 2 || shapeDraftCss.h >= 2) && (
        <g opacity={style.opacity ?? 1} pointerEvents="none">
          <ClosedShapeSvg
            kind={draft.shape}
            css={shapeDraftCss}
            fill={isNoneFill(style.fill) ? "transparent" : (style.fill ?? "#111827")}
            stroke={style.stroke ?? "#111827"}
            strokeWidth={style.strokeWidth ?? 1.5}
            locked
            interactive={false}
            onPointerDown={() => {}}
          />
        </g>
      )}
      {draft?.kind === "line" && (
        <line
          x1={draft.start.x}
          y1={draft.start.y}
          x2={draft.cur.x}
          y2={draft.cur.y}
          stroke={style.stroke ?? "#111827"}
          strokeWidth={style.strokeWidth ?? 2}
          pointerEvents="none"
        />
      )}
      {draft?.kind === "ink" && draft.points && draft.points.length > 1 && (
        <polyline
          points={draft.points.map((p) => `${p.x},${p.y}`).join(" ")}
          fill="none"
          stroke={style.stroke ?? "#111827"}
          strokeWidth={style.strokeWidth ?? 2.5}
          strokeLinecap="round"
          strokeLinejoin="round"
          pointerEvents="none"
        />
      )}
    </svg>
  );
}

function viewportRectFromCss(
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
  return {
    x: Math.min(...xs),
    y: Math.min(...ys),
    w: Math.max(...xs) - Math.min(...xs),
    h: Math.max(...ys) - Math.min(...ys),
  };
}

function ClosedShapeSvg({
  kind,
  css,
  fill,
  stroke,
  strokeWidth,
  locked,
  interactive,
  onPointerDown,
}: {
  kind: ClosedShapeKind;
  css: { x: number; y: number; w: number; h: number };
  fill: string;
  stroke: string;
  strokeWidth: number;
  locked: boolean;
  interactive: boolean;
  onPointerDown: (e: React.PointerEvent) => void;
}) {
  const common = {
    fill,
    stroke,
    strokeWidth,
    style: { cursor: !interactive ? undefined : locked ? "default" : "move" } as const,
    onPointerDown: interactive ? onPointerDown : undefined,
  };
  const w = Math.max(css.w, 1);
  const h = Math.max(css.h, 1);
  if (kind === "rect") {
    return <rect x={css.x} y={css.y} width={w} height={h} {...common} />;
  }
  if (kind === "roundRect") {
    const r = Math.min(w, h) * 0.18;
    return <rect x={css.x} y={css.y} width={w} height={h} rx={r} ry={r} {...common} />;
  }
  if (kind === "ellipse") {
    return <ellipse cx={css.x + w / 2} cy={css.y + h / 2} rx={w / 2} ry={h / 2} {...common} />;
  }
  if (kind === "bubble") {
    return <path d={bubbleSvgPath({ x: css.x, y: css.y, w, h })} {...common} />;
  }
  return (
    <polygon
      points={closedShapeCssPoints(kind, { x: css.x, y: css.y, w, h })}
      {...common}
    />
  );
}

function ObjectShape({
  obj,
  mapping,
  selected,
  selectedCount,
  interactive,
  onPointerDownObject,
  onPointerDownHandle,
  onPointerDownRotate,
}: {
  obj: EditObject;
  mapping: ViewportMapping;
  selected: boolean;
  selectedCount: number;
  interactive: boolean;
  onPointerDownObject: (e: React.PointerEvent, obj: EditObject) => void;
  onPointerDownHandle: (e: React.PointerEvent, obj: EditObject, handle: Handle) => void;
  onPointerDownRotate: (e: React.PointerEvent, obj: EditObject) => void;
}) {
  const css = pdfRectToViewport(obj.rect, mapping);
  const opacity = "opacity" in obj && typeof obj.opacity === "number" ? obj.opacity : 1;
  const rot = obj.objectRotate ?? 0;
  const mid = cssCenter(css);

  const moveCursor = interactive && !obj.locked ? "move" : undefined;

  return (
    <g
      transform={rot ? `rotate(${rot} ${mid.x} ${mid.y})` : undefined}
      style={{ pointerEvents: interactive ? "auto" : "none" }}
    >
      <g opacity={opacity}>
      {isClosedShapeObject(obj) && (
        <ClosedShapeSvg
          kind={obj.kind}
          css={css}
          fill={isNoneFill(obj.fill) ? "transparent" : (obj.fill ?? "#111827")}
          stroke={obj.stroke ?? "#111827"}
          strokeWidth={obj.strokeWidth ?? 1.5}
          locked={!!obj.locked}
          interactive={interactive}
          onPointerDown={(e) => onPointerDownObject(e, obj)}
        />
      )}
      {obj.kind === "text" && (
        <g style={{ cursor: moveCursor }} onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}>
          <rect
            x={css.x}
            y={css.y}
            width={Math.max(css.w, 1)}
            height={Math.max(css.h, 1)}
            fill="transparent"
            stroke="transparent"
            strokeWidth={1}
          />
          <foreignObject x={css.x} y={css.y} width={Math.max(css.w, 8)} height={Math.max(css.h, 8)}>
            <div
              className="pdf-editor__text-preview"
              style={{
                color: obj.color ?? "#111827",
                fontSize: Math.max(8, obj.fontSize * (mapping.cssHeight / Math.max(displayedSize(mapping.geometry).h, 1))),
                textAlign: obj.align ?? "left",
              }}
            >
              {obj.content || "Text"}
            </div>
          </foreignObject>
        </g>
      )}
      {obj.kind === "image" && (
        <g style={{ cursor: moveCursor }} onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}>
          {obj.previewUrl ? (
            <image
              href={obj.previewUrl}
              x={css.x}
              y={css.y}
              width={Math.max(css.w, 1)}
              height={Math.max(css.h, 1)}
              preserveAspectRatio={obj.keepAspect === false ? "none" : "xMidYMid meet"}
            />
          ) : (
            <rect x={css.x} y={css.y} width={Math.max(css.w, 1)} height={Math.max(css.h, 1)} fill="#e5e7eb" stroke="#9ca3af" />
          )}
        </g>
      )}
      {obj.kind === "line" && (
        <g style={{ cursor: moveCursor }} onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}>
          <line
            x1={pdfToViewport({ x: obj.x1, y: obj.y1 }, mapping).x}
            y1={pdfToViewport({ x: obj.x1, y: obj.y1 }, mapping).y}
            x2={pdfToViewport({ x: obj.x2, y: obj.y2 }, mapping).x}
            y2={pdfToViewport({ x: obj.x2, y: obj.y2 }, mapping).y}
            stroke={obj.stroke ?? "#111827"}
            strokeWidth={obj.strokeWidth ?? 2}
            strokeLinecap="round"
          />
          <line
            x1={pdfToViewport({ x: obj.x1, y: obj.y1 }, mapping).x}
            y1={pdfToViewport({ x: obj.x1, y: obj.y1 }, mapping).y}
            x2={pdfToViewport({ x: obj.x2, y: obj.y2 }, mapping).x}
            y2={pdfToViewport({ x: obj.x2, y: obj.y2 }, mapping).y}
            stroke="transparent"
            strokeWidth={12}
          />
        </g>
      )}
      {obj.kind === "ink" && (
        <polyline
          points={obj.points.map((p) => {
            const c = pdfToViewport(p, mapping);
            return `${c.x},${c.y}`;
          }).join(" ")}
          fill="none"
          stroke={obj.stroke ?? "#111827"}
          strokeWidth={obj.strokeWidth ?? 2.5}
          strokeLinecap="round"
          strokeLinejoin="round"
          style={{ cursor: moveCursor }}
          onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}
        />
      )}
      {obj.kind === "link" && (
        <g style={{ cursor: moveCursor }} onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}>
          <rect
            x={css.x}
            y={css.y}
            width={Math.max(css.w, 1)}
            height={Math.max(css.h, 1)}
            fill="transparent"
            stroke="#2563eb"
            strokeWidth={1.5}
            strokeDasharray="5 4"
          />
        </g>
      )}
      {obj.kind === "note" && (
        <g style={{ cursor: moveCursor }} onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}>
          <rect
            x={css.x}
            y={css.y}
            width={Math.max(css.w, 12)}
            height={Math.max(css.h, 12)}
            fill={obj.color ?? "#f59e0b"}
            stroke="#b45309"
            strokeWidth={1}
          />
        </g>
      )}
      {(obj.kind === "highlight" || obj.kind === "underline" || obj.kind === "strikeout") && (
        <g style={{ cursor: moveCursor }} onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}>
          {obj.kind === "highlight" ? (
            <rect
              x={css.x}
              y={css.y}
              width={Math.max(css.w, 1)}
              height={Math.max(css.h, 1)}
              fill={obj.color ?? "#facc15"}
              opacity={0.45}
            />
          ) : (
            <line
              x1={css.x}
              y1={obj.kind === "underline" ? css.y + Math.max(css.h, 1) - 2 : css.y + Math.max(css.h, 1) / 2}
              x2={css.x + Math.max(css.w, 1)}
              y2={obj.kind === "underline" ? css.y + Math.max(css.h, 1) - 2 : css.y + Math.max(css.h, 1) / 2}
              stroke={obj.color ?? (obj.kind === "underline" ? "#2563eb" : "#dc2626")}
              strokeWidth={2}
            />
          )}
        </g>
      )}
      {obj.kind === "markupInk" && (
        <g style={{ cursor: moveCursor }} onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}>
          {obj.strokes.map((stroke, i) => (
            <polyline
              key={i}
              points={stroke.map((p) => {
                const c = pdfToViewport(p, mapping);
                return `${c.x},${c.y}`;
              }).join(" ")}
              fill="none"
              stroke={obj.color ?? "#111827"}
              strokeWidth={2.5}
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          ))}
        </g>
      )}
      {obj.kind === "redact" && (
        <g style={{ cursor: moveCursor }} onPointerDown={interactive ? (e) => onPointerDownObject(e, obj) : undefined}>
          <rect
            x={css.x}
            y={css.y}
            width={Math.max(css.w, 1)}
            height={Math.max(css.h, 1)}
            fill={obj.fill ?? "#000000"}
            opacity={0.22}
          />
          <rect
            x={css.x}
            y={css.y}
            width={Math.max(css.w, 1)}
            height={Math.max(css.h, 1)}
            fill="url(#offpdf-redact-hatch)"
          />
          <rect
            x={css.x}
            y={css.y}
            width={Math.max(css.w, 1)}
            height={Math.max(css.h, 1)}
            fill="none"
            stroke="#b91c1c"
            strokeWidth={2}
          />
          {obj.label ? (
            <text
              x={css.x + 6}
              y={css.y + Math.max(14, Math.min(css.h - 4, 16))}
              fill="#7f1d1d"
              fontSize={11}
              fontFamily="ui-sans-serif, system-ui, sans-serif"
            >
              {obj.label}
            </text>
          ) : null}
        </g>
      )}
      </g>
      {selected && (
        <rect
          x={css.x - 3}
          y={css.y - 3}
          width={Math.max(css.w, 1) + 6}
          height={Math.max(css.h, 1) + 6}
          fill="none"
          stroke="#2563eb"
          strokeWidth={1}
          strokeDasharray="4 3"
          pointerEvents="none"
        />
      )}
      {selected && selectedCount === 1 && !obj.locked && obj.kind !== "link" && (
        <>
          <line
            x1={mid.x}
            y1={css.y - 3}
            x2={mid.x}
            y2={css.y - 22}
            stroke="#2563eb"
            strokeWidth={1}
            pointerEvents="none"
          />
          <circle
            cx={mid.x}
            cy={css.y - 26}
            r={6}
            fill="#fff"
            stroke="#2563eb"
            strokeWidth={1.5}
            style={{ cursor: "grab" }}
            onPointerDown={(e) => onPointerDownRotate(e, obj)}
          />
        </>
      )}
      {selected &&
        selectedCount === 1 &&
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
                cursor: handle === "nw" || handle === "se" ? "nwse-resize" : "nesw-resize",
              }}
              onPointerDown={(e) => onPointerDownHandle(e, obj, handle)}
            />
          );
        })}
    </g>
  );
}

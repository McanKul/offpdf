/**
 * SVG shapes for the editor overlay: closed shapes and one placed object with
 * its selection handles. Moved unchanged from `EditorOverlay.tsx`.
 */
import {
  bubbleSvgPath,
  closedShapeCssPoints,
  cssCenter,
  displayedSize,
  isClosedShapeObject,
  isNoneFill,
  pdfRectToViewport,
  pdfToViewport,
  type ClosedShapeKind,
  type EditObject,
  type ResizeHandle,
  type ViewportMapping,
} from "@/lib/editor";

type Handle = ResizeHandle;

export function ClosedShapeSvg({
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

export function ObjectShape({
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
      {selected && selectedCount === 1 && !obj.locked && obj.kind !== "link" && obj.kind !== "redact" && (
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

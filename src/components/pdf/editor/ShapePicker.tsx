import { useEffect, useRef } from "react";
import { Icon, type IconName } from "@/components/ui/Icon";
import { isClosedShape, type ClosedShapeKind } from "@/lib/editor";
import type { EditorTool } from "./EditorOverlay";

export const SHAPE_TOOLS: {
  id: Extract<
    EditorTool,
    | "line"
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
  >;
  label: string;
  icon: IconName;
}[] = [
  { id: "rect", label: "Rectangle — hold Shift for a square", icon: "rectangle" },
  { id: "square", label: "Square", icon: "square" },
  { id: "roundRect", label: "Rounded rectangle", icon: "roundSquare" },
  { id: "ellipse", label: "Ellipse — hold Shift for a circle", icon: "ellipse" },
  { id: "circle", label: "Circle", icon: "circle" },
  { id: "triangle", label: "Triangle", icon: "triangle" },
  { id: "star", label: "Star", icon: "star" },
  { id: "hexagon", label: "Hexagon", icon: "hexagon" },
  { id: "bubble", label: "Speech bubble", icon: "bubble" },
  { id: "arrow", label: "Arrow", icon: "arrowRight" },
  { id: "line", label: "Line", icon: "slash" },
];

export function isShapeTool(tool: EditorTool): boolean {
  return SHAPE_TOOLS.some((s) => s.id === tool);
}

export function shapeKindForTool(tool: EditorTool): ClosedShapeKind | null {
  if (tool === "square") return "rect";
  if (tool === "circle") return "ellipse";
  if (isClosedShape(tool)) return tool;
  return null;
}

export function toolForces1to1(tool: EditorTool): boolean {
  return tool === "square" || tool === "circle";
}

export function ShapePicker({
  tool,
  open,
  lastShape,
  onOpenChange,
  onPick,
}: {
  tool: EditorTool;
  open: boolean;
  lastShape: (typeof SHAPE_TOOLS)[number]["id"];
  onOpenChange: (open: boolean) => void;
  onPick: (id: (typeof SHAPE_TOOLS)[number]["id"]) => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const active = isShapeTool(tool);
  const shown =
    SHAPE_TOOLS.find((s) => s.id === (active ? tool : lastShape)) ??
    SHAPE_TOOLS.find((s) => s.id === "rect") ??
    SHAPE_TOOLS[0];

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) onOpenChange(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open, onOpenChange]);

  return (
    <div className="pdf-editor__shape-wrap" ref={wrapRef}>
      <button
        type="button"
        className={`btn btn--sm pdf-editor__shape-btn ${active ? "btn--primary" : "btn--ghost"}`}
        title="Shapes"
        aria-label="Shapes"
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={() => onOpenChange(!open)}
      >
        <Icon name={shown.icon} size={16} />
        <Icon name="chevronDown" size={12} />
      </button>
      {open && (
        <div className="pdf-editor__shape-menu" role="menu" aria-label="Shapes">
          {SHAPE_TOOLS.map((s) => (
            <button
              key={s.id}
              type="button"
              role="menuitem"
              title={s.label}
              aria-label={s.label}
              className={`pdf-editor__shape-item${tool === s.id ? " is-active" : ""}`}
              onClick={() => {
                onPick(s.id);
                onOpenChange(false);
              }}
            >
              <Icon name={s.icon} size={18} />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

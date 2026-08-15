import { isNearlySquare, type EditObject } from "@/lib/editor";

function labelFor(obj: EditObject, layer: string): string {
  const page = `p${obj.pageIndex + 1}`;
  if (obj.kind === "text") {
    const t = obj.content.trim() || "Text";
    return `Text: ${t.length > 18 ? `${t.slice(0, 18)}…` : t} · ${page} · ${layer}`;
  }
  if (obj.kind === "image") return `Image · ${page} · ${layer}`;
  if (obj.kind === "roundRect") return `Round rect · ${page} · ${layer}`;
  if (obj.kind === "ellipse") {
    return `${isNearlySquare(obj.rect) ? "Circle" : "Ellipse"} · ${page} · ${layer}`;
  }
  if (obj.kind === "triangle") return `Triangle · ${page} · ${layer}`;
  if (obj.kind === "star") return `Star · ${page} · ${layer}`;
  if (obj.kind === "hexagon") return `Hexagon · ${page} · ${layer}`;
  if (obj.kind === "bubble") return `Bubble · ${page} · ${layer}`;
  if (obj.kind === "arrow") return `Arrow · ${page} · ${layer}`;
  if (obj.kind === "line") return `Line · ${page} · ${layer}`;
  if (obj.kind === "ink") return `Drawing · ${page} · ${layer}`;
  return `${isNearlySquare(obj.rect) ? "Square" : "Rectangle"} · ${page} · ${layer}`;
}

export function ObjectList({
  objects,
  selectedIds,
  onSelect,
  onDelete,
}: {
  objects: EditObject[];
  selectedIds: string[];
  onSelect: (ids: string[]) => void;
  onDelete: (ids: string[]) => void;
}) {
  if (objects.length === 0) {
    return (
      <div className="pdf-editor__object-list muted" style={{ fontSize: 12.5 }}>
        No objects yet. Choose a tool and draw on the page.
      </div>
    );
  }

  return (
    <ul className="pdf-editor__object-list" role="listbox" aria-label="Edit objects">
      {objects.map((obj) => {
        const selected = selectedIds.includes(obj.id);
        const onPage = objects.filter((o) => o.pageIndex === obj.pageIndex);
        const z = onPage.findIndex((o) => o.id === obj.id) + 1;
        const layer = `${z}/${onPage.length}`;
        return (
          <li key={obj.id} role="option" aria-selected={selected}>
            <button
              type="button"
              className={`pdf-editor__object-item${selected ? " is-selected" : ""}`}
              onClick={(e) => {
                if (e.shiftKey || e.metaKey || e.ctrlKey) {
                  onSelect(
                    selected
                      ? selectedIds.filter((id) => id !== obj.id)
                      : [...selectedIds, obj.id],
                  );
                  return;
                }
                onSelect([obj.id]);
              }}
            >
              {labelFor(obj, layer)}
            </button>
            {selected && (
              <button
                type="button"
                className="pdf-editor__object-delete"
                aria-label={`Delete ${labelFor(obj, layer)}`}
                onClick={() => onDelete([obj.id])}
              >
                ✕
              </button>
            )}
          </li>
        );
      })}
    </ul>
  );
}

import type { EditObject } from "@/lib/editor";

function labelFor(obj: EditObject, index: number): string {
  const kind = obj.kind.charAt(0).toUpperCase() + obj.kind.slice(1);
  return `${kind} ${index + 1} · page ${obj.pageIndex + 1}`;
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
        No objects yet. Use “Add rectangle” or drag on the page.
      </div>
    );
  }

  return (
    <ul className="pdf-editor__object-list" role="listbox" aria-label="Edit objects">
      {objects.map((obj, i) => {
        const selected = selectedIds.includes(obj.id);
        return (
          <li key={obj.id} role="option" aria-selected={selected}>
            <button
              type="button"
              className={`pdf-editor__object-item${selected ? " is-selected" : ""}`}
              onClick={() => onSelect([obj.id])}
            >
              {labelFor(obj, i)}
            </button>
            {selected && (
              <button
                type="button"
                className="pdf-editor__object-delete"
                aria-label={`Delete ${labelFor(obj, i)}`}
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

import { Icon } from "@/components/ui/Icon";
import { Badge } from "@/components/ui/Badge";
import { formatBytes, fileSizeTier, formatCount } from "@/lib/formatBytes";
import { useSortable } from "./useSortable";
import { useFileThumb } from "./useFileThumb";
import type { WorkspaceFile } from "@/lib/types";

function SizeBadge({ bytes }: { bytes: number }) {
  const tier = fileSizeTier(bytes);
  if (tier === "veryLarge") return <Badge variant="warning">Very large</Badge>;
  if (tier === "large") return <Badge variant="info">Large</Badge>;
  return null;
}

function RowThumb({ path }: { path: string }) {
  const url = useFileThumb(path, 120);
  return (
    <div className="file-row__thumb">
      {url ? <img src={url} alt="" draggable={false} /> : <Icon name="fileText" size={16} />}
    </div>
  );
}

/** Drag-to-reorder file list used by Merge. Pointer-based (works in WKWebView
 * alongside Tauri's native file drop). Drag from the grip handle. */
export function SortableFileList({
  files,
  onReorder,
  onRemove,
}: {
  files: WorkspaceFile[];
  onReorder: (from: number, to: number) => void;
  onRemove: (index: number) => void;
}) {
  const { dragIndex, overIndex, begin } = useSortable(onReorder);

  return (
    <div className="file-list">
      {files.map((f, i) => (
        <div
          key={f.uid}
          data-sort-idx={i}
          className={`file-row ${dragIndex === i ? "is-dragging" : ""} ${
            overIndex === i && dragIndex !== i ? "is-over" : ""
          }`}
        >
          <div
            className="file-row__grip"
            title="Drag to reorder"
            onPointerDown={begin(i)}
            style={{ touchAction: "none" }}
          >
            <Icon name="grip" size={16} />
          </div>
          <div className="file-row__index">{i + 1}</div>
          <RowThumb path={f.path} />
          <div className="grow">
            <div className="file-row__name truncate" title={f.path}>
              {f.name}
            </div>
            <div className="file-row__meta">
              <span>{formatBytes(f.sizeBytes)}</span>
              {f.pageCount != null && <span>· {formatCount(f.pageCount)} pages</span>}
              <SizeBadge bytes={f.sizeBytes} />
            </div>
          </div>
          <button
            className="btn btn--ghost btn--sm"
            onClick={() => onRemove(i)}
            aria-label={`Remove ${f.name}`}
            title="Remove"
          >
            <Icon name="x" size={16} />
          </button>
        </div>
      ))}
    </div>
  );
}

import { Icon } from "@/components/ui/Icon";
import { Badge } from "@/components/ui/Badge";
import { formatBytes, fileSizeTier, formatCount } from "@/lib/formatBytes";
import type { FileInfo } from "@/lib/types";

function SizeBadge({ bytes }: { bytes: number }) {
  const tier = fileSizeTier(bytes);
  if (tier === "veryLarge") return <Badge variant="warning">Very large</Badge>;
  if (tier === "large") return <Badge variant="info">Large</Badge>;
  return null;
}

/** Simple, non-sortable file list (single-file tools or read-only display). */
export function FileList({
  files,
  onRemove,
}: {
  files: FileInfo[];
  onRemove?: (index: number) => void;
}) {
  return (
    <div className="file-list">
      {files.map((f, i) => (
        <div className="file-row" key={f.path}>
          <div className="file-row__icon">
            <Icon name="fileText" size={18} />
          </div>
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
          {onRemove && (
            <button
              className="btn btn--ghost btn--sm"
              onClick={() => onRemove(i)}
              aria-label={`Remove ${f.name}`}
              title="Remove"
            >
              <Icon name="x" size={16} />
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

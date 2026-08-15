import { Icon } from "@/components/ui/Icon";
import { Spinner } from "@/components/ui/Spinner";
import { useToast } from "@/components/ui/Toast";
import { Dropzone } from "./Dropzone";
import { useFileThumb } from "./useFileThumb";
import { useWorkspace } from "@/state/workspaceStore";
import { formatBytes, formatCount } from "@/lib/formatBytes";

function FileChip({
  path,
  name,
  size,
  pages,
  active,
  selectable,
  onSelect,
  onRemove,
}: {
  path: string;
  name: string;
  size: number;
  pages: number | null | undefined;
  active: boolean;
  selectable: boolean;
  onSelect: () => void;
  onRemove: () => void;
}) {
  const thumb = useFileThumb(path);
  return (
    <div
      className={`ws-file ${active && selectable ? "is-active" : ""}`}
      role={selectable ? "button" : undefined}
      tabIndex={selectable ? 0 : undefined}
      onClick={selectable ? onSelect : undefined}
      onKeyDown={(e) => {
        if (selectable && (e.key === "Enter" || e.key === " ")) onSelect();
      }}
      title={path}
    >
      <div className="ws-file__thumb">
        {thumb ? <img src={thumb} alt="" draggable={false} /> : <Icon name="fileText" size={18} />}
      </div>
      <div className="ws-file__info">
        <div className="ws-file__name truncate">{name}</div>
        <div className="ws-file__meta">
          {formatBytes(size)}
          {pages != null ? ` · ${formatCount(pages)}p` : ""}
        </div>
      </div>
      <button
        className="ws-file__x"
        title="Remove"
        aria-label={`Remove ${name}`}
        onClick={(e) => {
          e.stopPropagation();
          onRemove();
        }}
      >
        <Icon name="x" size={14} />
      </button>
    </div>
  );
}

/**
 * Shared document panel: shows the workspace files (with page-1 thumbnails),
 * lets you add more, remove, and — when `selectable` — pick the active file that
 * single-file tools act on. Loaded files persist across tools.
 */
export function WorkspaceFilePicker({
  selectable = true,
  onBeforeRemove,
}: {
  selectable?: boolean;
  /** Return false to keep the file. May be async (e.g. confirm discard). */
  onBeforeRemove?: (index: number) => boolean | Promise<boolean>;
}) {
  const files = useWorkspace((s) => s.files);
  const activeIndex = useWorkspace((s) => s.activeIndex);
  const addPaths = useWorkspace((s) => s.addPaths);
  const removeAt = useWorkspace((s) => s.removeAt);
  const setActive = useWorkspace((s) => s.setActive);
  const loading = useWorkspace((s) => s.loading);
  const { toast } = useToast();

  const remove = (index: number) => {
    void (async () => {
      if (onBeforeRemove && (await onBeforeRemove(index)) === false) return;
      removeAt(index);
    })();
  };

  const add = async (paths: string[]) => {
    const r = await addPaths(paths);
    if (r.notPdf) toast({ title: "Only PDF, image, or Office files are supported", variant: "error" });
    if (r.errors.length) {
      toast({ title: "Some files could not be added", description: r.errors.join(" · "), variant: "error" });
    }
    if (r.invalid.length) {
      toast({
        title: r.invalid.length === 1 ? "That file is not a valid PDF" : "Some files are not valid PDFs",
        description: r.invalid.join(", "),
        variant: "error",
      });
    }
  };

  if (files.length === 0) {
    return (
      <div className="col">
        <Dropzone multiple onFiles={add} />
        {loading && (
          <div className="row gap-sm muted" style={{ justifyContent: "center" }}>
            <Spinner /> Adding files…
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="ws-files">
      {files.map((f, i) => (
        <FileChip
          key={f.uid}
          path={f.path}
          name={f.name}
          size={f.sizeBytes}
          pages={f.pageCount}
          active={i === activeIndex}
          selectable={selectable}
          onSelect={() => setActive(i)}
          onRemove={() => remove(i)}
        />
      ))}
      {loading && (
        <div className="ws-file" style={{ alignItems: "center", justifyContent: "center" }}>
          <div className="ws-file__thumb">
            <Spinner />
          </div>
          <div className="ws-file__meta">Converting…</div>
        </div>
      )}
      <Dropzone multiple onFiles={add} compact />
    </div>
  );
}

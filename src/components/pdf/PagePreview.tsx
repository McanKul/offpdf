import { useEffect, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { PageLightbox } from "./PageLightbox";
import { useThumbnails } from "./useThumbnails";
import { rendererAvailable } from "@/lib/tauriCommands";
import type { FileInfo } from "@/lib/types";

const BATCH = 18;

function range(a: number, b: number): number[] {
  const out: number[] = [];
  for (let i = a; i <= b; i++) out.push(i);
  return out;
}

/** Read-only page preview gallery. Click a page to enlarge. Used by tools that
 * act on the whole document (merge/optimize/compress/split-everyN/range). */
export function PagePreview({ file }: { file: FileInfo }) {
  const total = file.pageCount ?? 0;
  const [shown, setShown] = useState(Math.min(BATCH, total));
  const [zoom, setZoom] = useState<number | null>(null);
  const [canRender, setCanRender] = useState(true);
  const { urls, ensure, isLoading } = useThumbnails(file);

  useEffect(() => {
    let active = true;
    rendererAvailable().then((v) => active && setCanRender(v)).catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    setShown(Math.min(BATCH, total));
  }, [file.path, total]);

  useEffect(() => {
    if (canRender) ensure(range(1, shown));
  }, [shown, ensure, canRender]);

  if (!canRender || total === 0) return null;

  return (
    <div className="col">
      <div className="thumb-grid">
        {range(1, shown).map((p) => (
          <div key={p} className="thumb" role="button" tabIndex={0} onClick={() => setZoom(p)}>
            <div className="thumb__img">
              {urls[p] ? (
                <img src={urls[p]} alt={`Page ${p}`} draggable={false} />
              ) : (
                <div className="thumb__ph">{isLoading(p) ? <Spinner /> : p}</div>
              )}
              <button
                className="thumb__zoom"
                title="Enlarge"
                onClick={(e) => {
                  e.stopPropagation();
                  setZoom(p);
                }}
              >
                <Icon name="external" size={13} />
              </button>
            </div>
            <div className="thumb__label">Page {p}</div>
          </div>
        ))}
      </div>

      {shown < total && (
        <div className="row" style={{ justifyContent: "center" }}>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => setShown((s) => Math.min(s + BATCH, total))}
          >
            Load more ({total - shown} left)
          </Button>
        </div>
      )}

      <PageLightbox
        file={file}
        page={zoom}
        pageCount={total}
        onChange={setZoom}
        onClose={() => setZoom(null)}
      />
    </div>
  );
}

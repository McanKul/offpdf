import { useEffect, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { Alert } from "@/components/ui/Alert";
import { RefLightbox } from "./RefLightbox";
import { PdfSearch } from "./PdfSearch";
import { useCombinedDoc } from "./useCombinedDoc";
import { useRefThumbnails } from "./useRefThumbnails";
import { rendererAvailable } from "@/lib/tauriCommands";
import type { PageRef } from "@/lib/types";

const BATCH = 24;

/** Read-only preview of the combined workspace document (all pages, all files).
 * Used by tools that act on the whole document (optimize/compress/split). */
export function CombinedPreview() {
  const refs = useCombinedDoc();
  const thumbs = useRefThumbnails();
  const [shown, setShown] = useState(BATCH);
  const [zoom, setZoom] = useState<PageRef | null>(null);
  const [canRender, setCanRender] = useState(true);

  useEffect(() => {
    let active = true;
    rendererAvailable().then((v) => active && setCanRender(v)).catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (canRender) thumbs.ensure(refs.slice(0, shown));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refs, shown, canRender]);

  if (refs.length === 0) return null;
  if (!canRender) {
    return (
      <Alert variant="info">
        Install poppler to see page previews. The operation still works without previews.
      </Alert>
    );
  }

  return (
    <div className="col">
      <PdfSearch refs={refs} onOpen={(r) => setZoom(r)} />
      <div className="thumb-grid">
        {refs.slice(0, shown).map((r, i) => (
          <div key={r.key} className="thumb" role="button" onClick={() => setZoom(r)}>
            <div className="thumb__img">
              {thumbs.get(r.key) ? (
                <img src={thumbs.get(r.key)} alt="" draggable={false} />
              ) : (
                <div className="thumb__ph">{thumbs.isLoading(r.key) ? <Spinner /> : i + 1}</div>
              )}
              <button
                className="thumb__zoom"
                title="Enlarge"
                onClick={(e) => {
                  e.stopPropagation();
                  setZoom(r);
                }}
              >
                <Icon name="external" size={13} />
              </button>
            </div>
            <div className="thumb__label truncate" title={`${r.fileName} · p${r.page}`}>
              {r.fileName} · p{r.page}
            </div>
          </div>
        ))}
      </div>

      {shown < refs.length && (
        <div className="row" style={{ justifyContent: "center" }}>
          <Button size="sm" variant="secondary" onClick={() => setShown((s) => s + BATCH)}>
            Load more ({refs.length - shown} left)
          </Button>
        </div>
      )}

      <RefLightbox list={refs} current={zoom} onClose={() => setZoom(null)} />
    </div>
  );
}

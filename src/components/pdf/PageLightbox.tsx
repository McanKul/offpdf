import { useEffect, useState } from "react";
import { Modal } from "@/components/ui/Modal";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { Icon } from "@/components/ui/Icon";
import { renderThumbnails } from "@/lib/tauriCommands";
import type { FileInfo } from "@/lib/types";

/** Enlarged single-page viewer with prev/next navigation (shared by pickers). */
export function PageLightbox({
  file,
  page,
  pageCount,
  onChange,
  onClose,
}: {
  file: FileInfo | undefined;
  page: number | null;
  pageCount: number;
  onChange: (page: number) => void;
  onClose: () => void;
}) {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    setUrl(null);
    if (!file || page == null) return;
    let active = true;
    renderThumbnails(file.path, [page], 1100)
      .then((res) => active && setUrl(res[0]?.dataUrl ?? null))
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [file, page]);

  useEffect(() => {
    if (page == null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowLeft" && page > 1) onChange(page - 1);
      if (e.key === "ArrowRight" && page < pageCount) onChange(page + 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [page, pageCount, onChange]);

  if (page == null) return null;

  return (
    <Modal
      open
      onClose={onClose}
      title={`Page ${page} of ${pageCount}`}
      footer={
        <div className="spread" style={{ width: "100%" }}>
          <Button
            variant="secondary"
            size="sm"
            disabled={page <= 1}
            onClick={() => onChange(page - 1)}
            leftIcon={<Icon name="chevronRight" size={14} style={{ transform: "rotate(180deg)" }} />}
          >
            Previous
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={page >= pageCount}
            onClick={() => onChange(page + 1)}
            rightIcon={<Icon name="chevronRight" size={14} />}
          >
            Next
          </Button>
        </div>
      }
    >
      <div style={{ display: "grid", placeItems: "center", minHeight: 280 }}>
        {url ? (
          <img
            src={url}
            alt={`Page ${page}`}
            style={{ maxWidth: "100%", maxHeight: "68vh", borderRadius: 8 }}
          />
        ) : (
          <Spinner />
        )}
      </div>
    </Modal>
  );
}

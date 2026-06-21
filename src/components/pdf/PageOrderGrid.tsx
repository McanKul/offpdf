import { useEffect } from "react";
import { Icon } from "@/components/ui/Icon";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { Alert } from "@/components/ui/Alert";
import { useSortable } from "./useSortable";
import type { FileInfo } from "@/lib/types";
import type { ThumbApi } from "./useThumbnails";

const MAX_VISUAL = 200;

/** Visual page picker (order mode): drag page tiles to set a new order. */
export function PageOrderGrid({
  file,
  value,
  onChange,
  onZoom,
  thumbs,
}: {
  file: FileInfo;
  value: number[];
  onChange: (order: number[]) => void;
  onZoom?: (page: number) => void;
  thumbs: ThumbApi;
}) {
  const total = file.pageCount ?? 0;
  const { urls, ensure, isLoading } = thumbs;

  const move = (from: number, to: number) => {
    const next = [...value];
    const [m] = next.splice(from, 1);
    next.splice(to, 0, m);
    onChange(next);
  };
  const { dragIndex, overIndex, begin } = useSortable(move);

  const tooBig = total > MAX_VISUAL;
  const shownOrder = value.slice(0, MAX_VISUAL);

  useEffect(() => {
    ensure(shownOrder);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, ensure]);

  return (
    <div className="col">
      {tooBig && (
        <Alert variant="info" title="Large document">
          Visual reordering is shown for the first {MAX_VISUAL} pages. For very large
          documents, the “Type order” tab is faster.
        </Alert>
      )}

      <div className="thumb-toolbar">
        <Button
          size="sm"
          variant="secondary"
          onClick={() => onChange(Array.from({ length: total }, (_, i) => i + 1))}
        >
          Reset order
        </Button>
        <Button size="sm" variant="ghost" onClick={() => onChange([...value].reverse())}>
          Reverse
        </Button>
        <span className="muted" style={{ marginLeft: "auto", fontSize: 12.5 }}>
          Drag tiles to reorder
        </span>
      </div>

      <div className="thumb-grid">
        {shownOrder.map((p, i) => (
          <div
            key={`${p}-${i}`}
            data-sort-idx={i}
            className={`thumb thumb--order ${dragIndex === i ? "is-dragging" : ""} ${
              overIndex === i && dragIndex !== i ? "is-over" : ""
            }`}
            onPointerDown={begin(i)}
          >
            <div className="thumb__img">
              {urls[p] ? (
                <img src={urls[p]} alt={`Page ${p}`} draggable={false} />
              ) : (
                <div className="thumb__ph">{isLoading(p) ? <Spinner /> : p}</div>
              )}
              <span className="thumb__order-index">{i + 1}</span>
              {onZoom && (
                <button
                  className="thumb__zoom"
                  title="Enlarge"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    onZoom(p);
                  }}
                >
                  <Icon name="external" size={13} />
                </button>
              )}
            </div>
            <div className="thumb__label">Page {p}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

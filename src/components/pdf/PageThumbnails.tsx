import { useEffect, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import type { FileInfo } from "@/lib/types";
import type { ThumbApi } from "./useThumbnails";

const BATCH = 24;

function range(a: number, b: number): number[] {
  const out: number[] = [];
  for (let i = a; i <= b; i++) out.push(i);
  return out;
}

/** Visual page picker (select mode): click pages to toggle them on/off. */
export function PageThumbnails({
  file,
  value,
  onChange,
  onZoom,
  thumbs,
}: {
  file: FileInfo;
  value: number[];
  onChange: (pages: number[]) => void;
  onZoom?: (page: number) => void;
  thumbs: ThumbApi;
}) {
  const total = file.pageCount ?? 0;
  const [shown, setShown] = useState(Math.min(BATCH, total));
  const { urls, ensure, isLoading } = thumbs;
  const selected = new Set(value);

  useEffect(() => {
    setShown(Math.min(BATCH, total));
  }, [file.path, total]);

  useEffect(() => {
    ensure(range(1, shown));
  }, [shown, ensure]);

  const toggle = (p: number) => {
    const s = new Set(value);
    if (s.has(p)) s.delete(p);
    else s.add(p);
    onChange([...s].sort((a, b) => a - b));
  };

  return (
    <div className="col">
      <div className="thumb-toolbar">
        <Button size="sm" variant="secondary" onClick={() => onChange(range(1, total))}>
          Select all
        </Button>
        <Button size="sm" variant="ghost" onClick={() => onChange([])}>
          Clear
        </Button>
        <span className="muted" style={{ marginLeft: "auto", fontSize: 12.5 }}>
          {value.length} of {total} selected
        </span>
      </div>

      <div className="thumb-grid">
        {range(1, shown).map((p) => (
          <div
            key={p}
            className={`thumb ${selected.has(p) ? "is-selected" : ""}`}
            role="button"
            tabIndex={0}
            onClick={() => toggle(p)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                toggle(p);
              }
            }}
          >
            <div className="thumb__img">
              {urls[p] ? (
                <img src={urls[p]} alt={`Page ${p}`} draggable={false} />
              ) : (
                <div className="thumb__ph">{isLoading(p) ? <Spinner /> : p}</div>
              )}
              <span className="thumb__check">
                <Icon name="check" size={13} />
              </span>
              {onZoom && (
                <button
                  className="thumb__zoom"
                  title="Enlarge"
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
    </div>
  );
}

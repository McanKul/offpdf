import { useEffect, useRef, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { Alert } from "@/components/ui/Alert";
import { RefLightbox } from "./RefLightbox";
import { PdfSearch } from "./PdfSearch";
import { useCombinedDoc, buildGroups } from "./useCombinedDoc";
import { useRefThumbnails } from "./useRefThumbnails";
import { useSortable } from "./useSortable";
import { formatPageList } from "@/lib/pageRange";
import type { PageGroup, PageRef, RotateGroup } from "@/lib/types";

const BATCH = 60;

interface Snapshot {
  order: PageRef[];
  rotation: Record<string, number>;
}

/**
 * Direct-manipulation page editor over the combined workspace document:
 * drag to reorder, ✕ to delete a page, ⟳ to rotate a page, and Undo to restore.
 * Reports the resulting plan (kept pages in order + per-page rotations).
 */
export function PageEditor({
  onChange,
}: {
  onChange: (groups: PageGroup[], rotations: RotateGroup[], keptCount: number) => void;
}) {
  const refs = useCombinedDoc();
  const thumbs = useRefThumbnails();
  const [order, setOrder] = useState<PageRef[]>(refs);
  const [rotation, setRotation] = useState<Record<string, number>>({});
  const [history, setHistory] = useState<Snapshot[]>([]);
  const [shown, setShown] = useState(BATCH);
  const [zoom, setZoom] = useState<PageRef | null>(null);
  const prevKeys = useRef<Set<string>>(new Set());

  // Reconcile with the workspace: drop pages whose file was removed, append the
  // pages of newly-added files (never re-add a page the user deleted). We
  // compute the "new since last time" keys BEFORE updating the ref, and capture
  // them in the updater, so state-update timing can't drop pages.
  useEffect(() => {
    const live = new Set(refs.map((r) => r.key));
    const newRefs = refs.filter((r) => !prevKeys.current.has(r.key));
    setOrder((prev) => {
      const kept = prev.filter((r) => live.has(r.key));
      const keptKeys = new Set(kept.map((r) => r.key));
      const toAdd = newRefs.filter((r) => !keptKeys.has(r.key));
      if (toAdd.length === 0 && kept.length === prev.length) return prev;
      return [...kept, ...toAdd];
    });
    prevKeys.current = live;
  }, [refs]);

  useEffect(() => {
    thumbs.ensure(order.slice(0, shown));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [order, shown]);

  // Emit the plan whenever order/rotation changes.
  const cb = useRef(onChange);
  cb.current = onChange;
  useEffect(() => {
    const groups = buildGroups(order);
    const byAngle = new Map<number, number[]>();
    order.forEach((r, i) => {
      const deg = ((rotation[r.key] ?? 0) % 360 + 360) % 360;
      if (deg !== 0) {
        const arr = byAngle.get(deg) ?? [];
        arr.push(i + 1);
        byAngle.set(deg, arr);
      }
    });
    const rotations: RotateGroup[] = [...byAngle.entries()].map(([angle, pages]) => ({
      angle,
      pages: formatPageList(pages),
    }));
    cb.current(groups, rotations, order.length);
  }, [order, rotation]);

  const snapshot = () => {
    setHistory((h) => [...h.slice(-99), { order: [...order], rotation: { ...rotation } }]);
  };

  const move = (from: number, to: number) => {
    snapshot();
    setOrder((prev) => {
      const next = [...prev];
      const [m] = next.splice(from, 1);
      next.splice(to, 0, m);
      return next;
    });
  };
  const { dragIndex, overIndex, begin } = useSortable(move);

  const removePage = (i: number) => {
    snapshot();
    setOrder((prev) => prev.filter((_, idx) => idx !== i));
  };
  const rotatePage = (key: string) => {
    snapshot();
    setRotation((prev) => ({ ...prev, [key]: ((prev[key] ?? 0) + 90) % 360 }));
  };
  const rotateAll = () => {
    snapshot();
    setRotation((prev) => {
      const next = { ...prev };
      for (const r of order) next[r.key] = ((next[r.key] ?? 0) + 90) % 360;
      return next;
    });
  };
  const reset = () => {
    snapshot();
    setOrder(refs);
    setRotation({});
  };
  const undo = () => {
    setHistory((h) => {
      if (h.length === 0) return h;
      const last = h[h.length - 1];
      setOrder(last.order);
      setRotation(last.rotation);
      return h.slice(0, -1);
    });
  };

  if (refs.length === 0) {
    return <Alert variant="info">Add a PDF or image above to start editing pages.</Alert>;
  }

  const removed = refs.length - order.length;
  const list = order.slice(0, shown);

  return (
    <div className="col">
      <div className="thumb-toolbar wrap">
        <Button size="sm" variant="secondary" onClick={undo} disabled={history.length === 0} leftIcon={<Icon name="undo" size={14} />}>
          Undo
        </Button>
        <Button size="sm" variant="ghost" onClick={rotateAll} leftIcon={<Icon name="rotate" size={14} />}>
          Rotate all
        </Button>
        <Button size="sm" variant="ghost" onClick={reset}>
          Reset
        </Button>
        <span className="muted" style={{ marginLeft: "auto", fontSize: 12.5 }}>
          {order.length} pages{removed > 0 ? ` · ${removed} removed` : ""} · drag to reorder
        </span>
      </div>

      <PdfSearch refs={order} onOpen={(r) => setZoom(r)} />

      <div className="thumb-grid">
        {list.map((r, i) => {
          const deg = rotation[r.key] ?? 0;
          return (
            <div
              key={r.key}
              data-sort-idx={i}
              className={`thumb thumb--order ${dragIndex === i ? "is-dragging" : ""} ${
                overIndex === i && dragIndex !== i ? "is-over" : ""
              }`}
              onPointerDown={begin(i)}
            >
              <div className="thumb__img">
                {thumbs.get(r.key) ? (
                  <img
                    src={thumbs.get(r.key)}
                    alt=""
                    draggable={false}
                    style={{ transform: `rotate(${deg}deg)`, transition: "transform 0.15s" }}
                  />
                ) : (
                  <div className="thumb__ph">{thumbs.isLoading(r.key) ? <Spinner /> : i + 1}</div>
                )}
                <span className="thumb__order-index">{i + 1}</span>
                <div className="thumb__btns" onPointerDown={(e) => e.stopPropagation()}>
                  <button className="thumb__btn" title="Rotate 90°" onClick={() => rotatePage(r.key)}>
                    <Icon name="rotate" size={13} />
                  </button>
                  <button className="thumb__btn" title="Enlarge" onClick={() => setZoom(r)}>
                    <Icon name="external" size={13} />
                  </button>
                  <button className="thumb__btn thumb__btn--danger" title="Delete page" onClick={() => removePage(i)}>
                    <Icon name="x" size={13} />
                  </button>
                </div>
              </div>
              <div className="thumb__label truncate" title={`${r.fileName} · p${r.page}`}>
                {r.fileName} · p{r.page}
              </div>
            </div>
          );
        })}
      </div>

      {shown < order.length && (
        <div className="row" style={{ justifyContent: "center" }}>
          <Button size="sm" variant="secondary" onClick={() => setShown((s) => s + BATCH)}>
            Load more ({order.length - shown} left)
          </Button>
        </div>
      )}

      {order.length === 0 && (
        <Alert variant="warning">
          All pages are removed. Use Undo or Reset to bring pages back.
        </Alert>
      )}

      <RefLightbox list={order} current={zoom} onClose={() => setZoom(null)} />
    </div>
  );
}

import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { Tabs } from "@/components/ui/Tabs";
import { Alert } from "@/components/ui/Alert";
import { RefLightbox } from "./RefLightbox";
import { useCombinedDoc, buildGroups } from "./useCombinedDoc";
import { useRefThumbnails } from "./useRefThumbnails";
import { useSortable } from "./useSortable";
import { parsePageRange, formatPageList } from "@/lib/pageRange";
import { rendererAvailable } from "@/lib/tauriCommands";
import type { PageGroup, PageRef } from "@/lib/types";

const BATCH = 30;

export type ArrangeMode = "order" | "keep" | "delete";

/**
 * Cross-document page editor. Shows every page of every loaded file as one
 * document. `order` = drag to set the page order; `keep` = click pages to keep;
 * `delete` = click pages to remove. Reports the resulting plan (qpdf groups)
 * and how many pages the output will have.
 */
export function CombinedArrange({
  mode,
  onPlan,
}: {
  mode: ArrangeMode;
  onPlan: (groups: PageGroup[], outRefs: PageRef[]) => void;
}) {
  const refs = useCombinedDoc();
  const thumbs = useRefThumbnails();
  const [canRender, setCanRender] = useState(true);
  const [tab, setTab] = useState<"visual" | "type">("visual");
  const userPicked = useRef(false);
  const [zoom, setZoom] = useState<PageRef | null>(null);
  const [shown, setShown] = useState(BATCH);

  // order mode state
  const [order, setOrder] = useState<PageRef[]>(refs);
  // select mode state (keys)
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [text, setText] = useState("");

  useEffect(() => {
    let active = true;
    rendererAvailable().then((v) => active && setCanRender(v)).catch(() => {});
    return () => {
      active = false;
    };
  }, []);
  useEffect(() => {
    if (canRender && !userPicked.current) setTab("visual");
    else if (!canRender) setTab("type");
  }, [canRender]);

  // Reconcile order with the workspace: keep existing order, drop removed pages,
  // append newly-added files' pages. This is how added PDFs show up here.
  useEffect(() => {
    setOrder((prev) => {
      const live = new Set(refs.map((r) => r.key));
      const kept = prev.filter((r) => live.has(r.key));
      const present = new Set(kept.map((r) => r.key));
      const added = refs.filter((r) => !present.has(r.key));
      const next = [...kept, ...added];
      if (next.length === prev.length && next.every((r, i) => r.key === prev[i].key)) return prev;
      return next;
    });
  }, [refs]);

  // Drop selections for files that were removed.
  useEffect(() => {
    setSel((prev) => {
      const live = new Set(refs.map((r) => r.key));
      const next = new Set([...prev].filter((k) => live.has(k)));
      return next.size === prev.size ? prev : next;
    });
  }, [refs]);

  const outRefs = useMemo(() => {
    if (mode === "order") return order;
    if (mode === "keep") return refs.filter((r) => sel.has(r.key));
    return refs.filter((r) => !sel.has(r.key)); // delete
  }, [mode, order, sel, refs]);

  // Ref-based so an inline onPlan prop can't cause an update loop; fires only
  // when the resulting plan actually changes.
  const planCb = useRef(onPlan);
  planCb.current = onPlan;
  useEffect(() => {
    planCb.current(buildGroups(outRefs), outRefs);
  }, [outRefs]);

  // Render thumbnails for whatever is visible.
  const visibleList = mode === "order" ? order : refs;
  useEffect(() => {
    if (canRender) thumbs.ensure(visibleList.slice(0, shown));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visibleList, shown, canRender]);

  // ---- drag reorder (order mode) ----
  const move = (from: number, to: number) => {
    setOrder((prev) => {
      const next = [...prev];
      const [m] = next.splice(from, 1);
      next.splice(to, 0, m);
      return next;
    });
  };
  const { dragIndex, overIndex, begin } = useSortable(move);

  // ---- type mode (global indices over the combined doc) ----
  const applyText = (v: string) => {
    setText(v);
    const res = parsePageRange(v, {
      pageCount: refs.length,
      preserveOrder: mode === "order",
      allowAll: true,
    });
    if (!res.ok) return;
    if (mode === "order") {
      setOrder(res.pages.map((i) => refs[i - 1]).filter(Boolean) as PageRef[]);
    } else {
      setSel(new Set(res.pages.map((i) => refs[i - 1]?.key).filter(Boolean) as string[]));
    }
  };
  const enterType = () => {
    userPicked.current = true;
    // seed the text from the current selection/order as global indices
    const idxByKey = new Map(refs.map((r, i) => [r.key, i + 1]));
    const chosen = mode === "order" ? order : mode === "keep" ? refs.filter((r) => sel.has(r.key)) : refs.filter((r) => sel.has(r.key));
    setText(formatPageList(chosen.map((r) => idxByKey.get(r.key)!).filter(Boolean)));
    setTab("type");
  };

  if (refs.length === 0) {
    return <Alert variant="info">Add a PDF or image above to start.</Alert>;
  }

  const toggle = (key: string) => {
    setSel((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const selectedCount = mode === "order" ? order.length : sel.size;
  const list = (mode === "order" ? order : refs).slice(0, shown);

  return (
    <div className="col">
      <div className="spread" style={{ gap: 12 }}>
        <span className="muted" style={{ fontSize: 12.5 }}>
          {mode === "order"
            ? `${order.length} pages · drag to reorder`
            : mode === "keep"
              ? `${sel.size} of ${refs.length} pages kept`
              : `${refs.length - sel.size} of ${refs.length} pages remain`}
        </span>
        {canRender && (
          <Tabs
            tabs={[
              { id: "visual", label: mode === "order" ? "Reorder visually" : "Pick visually" },
              { id: "type", label: "Type" },
            ]}
            active={tab}
            onChange={(t) => (t === "type" ? enterType() : (userPicked.current = true, setTab("visual")))}
          />
        )}
      </div>

      <div className="thumb-toolbar">
        {mode === "order" ? (
          <>
            <Button size="sm" variant="secondary" onClick={() => setOrder(refs)}>
              Reset order
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setOrder((o) => [...o].reverse())}>
              Reverse
            </Button>
          </>
        ) : (
          <>
            <Button size="sm" variant="secondary" onClick={() => setSel(new Set(refs.map((r) => r.key)))}>
              {mode === "keep" ? "Keep all" : "Select all"}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setSel(new Set())}>
              Clear
            </Button>
          </>
        )}
      </div>

      {tab === "type" || !canRender ? (
        <div className="field">
          <input
            className="input"
            value={text}
            onChange={(e) => applyText(e.target.value)}
            placeholder={`Global page numbers 1–${refs.length}, e.g. 1,3,2`}
            spellCheck={false}
          />
          <span className="field__hint">
            Numbers refer to the combined document ({refs.length} pages across {new Set(refs.map((r) => r.path)).size} files).
          </span>
        </div>
      ) : (
        <>
          <div className="thumb-grid">
            {list.map((r, i) => {
              const selectedForStyle =
                mode === "order" ? true : mode === "keep" ? sel.has(r.key) : !sel.has(r.key);
              const dimmed = mode !== "order" && !selectedForStyle;
              return (
                <div
                  key={r.key}
                  data-sort-idx={mode === "order" ? i : undefined}
                  className={`thumb ${mode === "order" ? "thumb--order" : ""} ${
                    mode !== "order" && selectedForStyle ? "is-selected" : ""
                  } ${dragIndex === i ? "is-dragging" : ""} ${
                    overIndex === i && dragIndex !== i ? "is-over" : ""
                  }`}
                  role={mode !== "order" ? "button" : undefined}
                  onPointerDown={mode === "order" ? begin(i) : undefined}
                  onClick={mode !== "order" ? () => toggle(r.key) : undefined}
                  style={dimmed ? { opacity: 0.5 } : undefined}
                >
                  <div className="thumb__img">
                    {thumbs.get(r.key) ? (
                      <img src={thumbs.get(r.key)} alt="" draggable={false} />
                    ) : (
                      <div className="thumb__ph">{thumbs.isLoading(r.key) ? <Spinner /> : i + 1}</div>
                    )}
                    {mode === "order" ? (
                      <span className="thumb__order-index">{i + 1}</span>
                    ) : (
                      <span className="thumb__check">
                        <Icon name="check" size={13} />
                      </span>
                    )}
                    <button
                      className="thumb__zoom"
                      title="Enlarge"
                      onPointerDown={(e) => e.stopPropagation()}
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
              );
            })}
          </div>
          {shown < (mode === "order" ? order.length : refs.length) && (
            <div className="row" style={{ justifyContent: "center" }}>
              <Button size="sm" variant="secondary" onClick={() => setShown((s) => s + BATCH)}>
                Load more ({(mode === "order" ? order.length : refs.length) - shown} left)
              </Button>
            </div>
          )}
        </>
      )}

      {selectedCount === 0 && mode !== "order" && (
        <span className="field__hint">Select at least one page.</span>
      )}

      <RefLightbox list={mode === "order" ? order : refs} current={zoom} onClose={() => setZoom(null)} />
    </div>
  );
}

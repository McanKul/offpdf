import { useEffect, useMemo, useRef, useState } from "react";
import { Modal } from "@/components/ui/Modal";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { Icon } from "@/components/ui/Icon";
import { renderThumbnails, pagePdf, pdfOutline } from "@/lib/tauriCommands";
import { base64ToBytes } from "@/lib/pdfjs";
import { ensureText, pageText, isLoaded } from "@/lib/docText";
import { PdfVectorView } from "./PdfVectorView";
import { useRefThumbnails } from "./useRefThumbnails";
import type { OutlineItem, PageRef } from "@/lib/types";

const outlineCache = new Map<string, OutlineItem[]>();

type ViewMode = "loading" | "vector" | "raster";

const MIN_ZOOM = 1;
const MAX_ZOOM = 5;
const STEP = 0.25;
/** Max long-side render resolution (must match the backend clamp). */
const MAX_RENDER = 6000;

/**
 * Full-page reader: page-thumbnail strip on the left, find-in-document bar on
 * top, and the page rendered as true vector (pdf.js) with raster fallback.
 * Navigates the whole provided list (the combined document).
 */
export function RefLightbox({
  list,
  current,
  onClose,
}: {
  list: PageRef[];
  current: PageRef | null;
  onClose: () => void;
}) {
  const [index, setIndex] = useState(0);
  const [url, setUrl] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const [panning, setPanning] = useState(false);
  const [mode, setMode] = useState<ViewMode>("loading");
  const [bytes, setBytes] = useState<Uint8Array | null>(null);
  const [query, setQuery] = useState("");
  const [matchPos, setMatchPos] = useState(0);
  const [textVer, setTextVer] = useState(0);
  const [sideTab, setSideTab] = useState<"pages" | "outline">("pages");
  const [outline, setOutline] = useState<OutlineItem[]>([]);

  const scroller = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; sl: number; st: number } | null>(null);
  const renderedTag = useRef<string>("");
  const thumbs = useRefThumbnails(180);

  const clamp = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(z * 100) / 100));
  const q = query.trim();

  // Jump to the opened page's position in the list.
  useEffect(() => {
    if (!current) return;
    const i = list.findIndex((r) => r.key === current.key);
    setIndex(i >= 0 ? i : 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current?.key]);

  const ref = current ? list[index] : null;

  // Reset zoom on page change.
  useEffect(() => {
    setZoom(1);
  }, [ref?.key]);

  // Prefer true-vector (pdf.js single page); fall back to raster.
  useEffect(() => {
    if (!ref) return;
    let active = true;
    setMode("loading");
    setBytes(null);
    pagePdf(ref.path, ref.page)
      .then((b64) => {
        if (!active) return;
        if (b64) {
          setBytes(base64ToBytes(b64));
          setMode("vector");
        } else setMode("raster");
      })
      .catch(() => active && setMode("raster"));
    return () => {
      active = false;
    };
  }, [ref?.key]);

  // Raster fallback render at the displayed resolution.
  useEffect(() => {
    if (!ref || mode !== "raster") return;
    const dpr = window.devicePixelRatio || 1;
    const vh = window.innerHeight || 900;
    const target = Math.min(MAX_RENDER, Math.max(1400, Math.round(vh * 0.8 * zoom * dpr)));
    const tag = `${ref.key}@${target}`;
    if (tag === renderedTag.current) return;
    const pageChanged = !renderedTag.current.startsWith(`${ref.key}@`);
    if (pageChanged) setUrl(null);
    let active = true;
    const t = setTimeout(
      () => {
        renderThumbnails(ref.path, [ref.page], target)
          .then((res) => {
            if (active && res[0]) {
              setUrl(res[0].dataUrl);
              renderedTag.current = tag;
            }
          })
          .catch(() => {});
      },
      pageChanged ? 0 : 200,
    );
    return () => {
      active = false;
      clearTimeout(t);
    };
  }, [ref?.key, zoom, mode]);

  // Keyboard nav + zoom (ignore arrows while typing in the search box).
  useEffect(() => {
    if (!current) return;
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement)?.tagName === "INPUT";
      if (!typing && e.key === "ArrowLeft") setIndex((i) => Math.max(0, i - 1));
      if (!typing && e.key === "ArrowRight") setIndex((i) => Math.min(list.length - 1, i + 1));
      if (e.key === "+" || e.key === "=") setZoom((z) => clamp(z + STEP));
      if (e.key === "-" || e.key === "_") setZoom((z) => clamp(z - STEP));
      if (e.key === "0") setZoom(1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [current, list.length]);

  // Load text for search when the user starts typing.
  useEffect(() => {
    if (q.length < 2 || isLoaded(list.map((r) => r.path))) return;
    let active = true;
    ensureText(list.map((r) => r.path)).then(() => active && setTextVer((v) => v + 1));
    return () => {
      active = false;
    };
  }, [q, list]);

  // Matching page indices for the current query.
  const matches = useMemo(() => {
    if (q.length < 2) return [];
    const needle = q.toLowerCase();
    const hits: number[] = [];
    for (let i = 0; i < list.length; i++) {
      if (pageText(list[i].path, list[i].page).toLowerCase().includes(needle)) hits.push(i);
    }
    return hits;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q, list, textVer]);

  // When matches change, jump to the first one.
  useEffect(() => {
    if (matches.length > 0) {
      setMatchPos(0);
      setIndex(matches[0]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [matches.length, q]);

  const gotoMatch = (delta: number) => {
    if (matches.length === 0) return;
    const pos = (matchPos + delta + matches.length) % matches.length;
    setMatchPos(pos);
    setIndex(matches[pos]);
  };

  // Keep the page-strip thumbnails around the current page rendered.
  useEffect(() => {
    const start = Math.max(0, index - 12);
    const end = Math.min(list.length, index + 24);
    thumbs.ensure(list.slice(start, end));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [index, list]);

  // Load the outline of the file the current page belongs to (cached per path).
  useEffect(() => {
    if (!ref) return;
    const path = ref.path;
    const cached = outlineCache.get(path);
    if (cached) {
      setOutline(cached);
      return;
    }
    let active = true;
    pdfOutline(path)
      .then((items) => {
        outlineCache.set(path, items);
        if (active) setOutline(items);
      })
      .catch(() => active && setOutline([]));
    return () => {
      active = false;
    };
  }, [ref?.path]);

  // Jump to an outline entry (page within the current file).
  const gotoOutline = (it: OutlineItem) => {
    if (!ref || it.page == null) return;
    const i = list.findIndex((r) => r.path === ref.path && r.page === it.page);
    if (i >= 0) setIndex(i);
  };

  if (!current || !ref) return null;

  const pct = Math.round(zoom * 100);

  return (
    <Modal
      open
      wide
      onClose={onClose}
      title={`${ref.fileName} · page ${index + 1} of ${list.length}`}
      headerActions={
        <div className="row" style={{ gap: 6 }}>
          <button
            className="btn btn--ghost btn--sm"
            title="Zoom out (-)"
            aria-label="Zoom out"
            disabled={zoom <= MIN_ZOOM}
            onClick={() => setZoom((z) => clamp(z - STEP))}
          >
            <Icon name="minus" size={15} />
          </button>
          <button
            className="btn btn--ghost btn--sm"
            title="Reset (0)"
            aria-label={`Reset zoom, current zoom ${pct}%`}
            onClick={() => setZoom(1)}
            style={{ minWidth: 52 }}
          >
            {pct}%
          </button>
          <button
            className="btn btn--ghost btn--sm"
            title="Zoom in (+)"
            aria-label="Zoom in"
            disabled={zoom >= MAX_ZOOM}
            onClick={() => setZoom((z) => clamp(z + STEP))}
          >
            <Icon name="plus" size={15} />
          </button>
        </div>
      }
      footer={
        <div className="spread" style={{ width: "100%" }}>
          <Button variant="secondary" size="sm" disabled={index <= 0} onClick={() => setIndex((i) => Math.max(0, i - 1))} leftIcon={<Icon name="chevronRight" size={14} style={{ transform: "rotate(180deg)" }} />}>
            Previous
          </Button>
          <span className="muted" style={{ fontSize: 12.5 }}>
            {index + 1} / {list.length}
          </span>
          <Button variant="secondary" size="sm" disabled={index >= list.length - 1} onClick={() => setIndex((i) => Math.min(list.length - 1, i + 1))} rightIcon={<Icon name="chevronRight" size={14} />}>
            Next
          </Button>
        </div>
      }
    >
      <div className="reader">
        {/* Left: tabs + page strip or outline */}
        <aside className="reader__side">
          <div className="reader__tabs">
            <button className={`reader__tab ${sideTab === "pages" ? "is-active" : ""}`} onClick={() => setSideTab("pages")}>
              Pages
            </button>
            <button
              className={`reader__tab ${sideTab === "outline" ? "is-active" : ""}`}
              onClick={() => setSideTab("outline")}
              disabled={outline.length === 0}
              title={outline.length === 0 ? "No bookmarks in this file" : "Contents"}
            >
              Contents
            </button>
          </div>

          {sideTab === "pages" ? (
            <div className="reader__strip">
              {list.map((r, i) => (
                <button
                  key={r.key}
                  className={`reader__thumb ${i === index ? "is-active" : ""}`}
                  onClick={() => setIndex(i)}
                  title={`Page ${i + 1}`}
                  aria-label={`Page ${i + 1}`}
                  aria-current={i === index ? "page" : undefined}
                >
                  {thumbs.get(r.key) ? (
                    <img src={thumbs.get(r.key)} alt="" draggable={false} />
                  ) : (
                    <div className="reader__thumb-ph">{i + 1}</div>
                  )}
                  <span className="reader__thumb-no">{i + 1}</span>
                </button>
              ))}
            </div>
          ) : (
            <div className="reader__outline">
              {outline.map((it, i) => (
                <button
                  key={`${i}-${it.title}`}
                  className="reader__ol-item"
                  style={{ paddingLeft: 8 + it.level * 12 }}
                  disabled={it.page == null}
                  onClick={() => gotoOutline(it)}
                  title={it.title}
                >
                  <span className="truncate">{it.title}</span>
                  {it.page != null && <span className="reader__ol-page">{it.page}</span>}
                </button>
              ))}
            </div>
          )}
        </aside>

        {/* Right: search bar + page */}
        <div className="reader__main">
          <div className="reader__find">
            <Icon name="info" size={15} className="subtle" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") gotoMatch(e.shiftKey ? -1 : 1);
              }}
              aria-label="Find in document"
              placeholder="Find in document…"
              spellCheck={false}
            />
            {q.length >= 2 && (
              <span className="muted" style={{ fontSize: 12, whiteSpace: "nowrap" }}>
                {matches.length === 0 ? "0 / 0" : `${matchPos + 1} / ${matches.length}`}
              </span>
            )}
            <button className="btn btn--ghost btn--sm" title="Previous match (Shift+Enter)" aria-label="Previous match" disabled={matches.length === 0} onClick={() => gotoMatch(-1)}>
              <Icon name="chevronRight" size={14} style={{ transform: "rotate(180deg)" }} />
            </button>
            <button className="btn btn--ghost btn--sm" title="Next match (Enter)" aria-label="Next match" disabled={matches.length === 0} onClick={() => gotoMatch(1)}>
              <Icon name="chevronRight" size={14} />
            </button>
            {query && (
              <button className="btn btn--ghost btn--sm" title="Clear" aria-label="Clear search" onClick={() => setQuery("")}>
                <Icon name="x" size={14} />
              </button>
            )}
          </div>

          <div
            ref={scroller}
            className="reader__canvas"
            style={{ cursor: panning ? "grabbing" : "grab", alignItems: zoom > 1 ? "flex-start" : "center" }}
            onWheel={(e) => {
              if (e.ctrlKey || e.metaKey) {
                e.preventDefault();
                setZoom((z) => clamp(z + (e.deltaY < 0 ? STEP : -STEP)));
              }
            }}
            onDoubleClick={() => setZoom((z) => (z > 1 ? 1 : 2))}
            onMouseDown={(e) => {
              if (zoom <= 1 || !scroller.current) return;
              drag.current = { x: e.clientX, y: e.clientY, sl: scroller.current.scrollLeft, st: scroller.current.scrollTop };
              setPanning(true);
            }}
            onMouseMove={(e) => {
              if (!drag.current || !scroller.current) return;
              scroller.current.scrollLeft = drag.current.sl - (e.clientX - drag.current.x);
              scroller.current.scrollTop = drag.current.st - (e.clientY - drag.current.y);
            }}
            onMouseUp={() => {
              drag.current = null;
              setPanning(false);
            }}
            onMouseLeave={() => {
              drag.current = null;
              setPanning(false);
            }}
          >
            {mode === "vector" && bytes ? (
              <PdfVectorView bytes={bytes} zoom={zoom} highlight={q.length >= 2 ? q : undefined} onFail={() => setMode("raster")} />
            ) : mode === "raster" && url ? (
              <img src={url} alt="" draggable={false} style={{ height: `${78 * zoom}vh`, width: "auto", maxWidth: zoom <= 1 ? "100%" : "none", borderRadius: 8 }} />
            ) : (
              <Spinner />
            )}
          </div>
        </div>
      </div>
    </Modal>
  );
}

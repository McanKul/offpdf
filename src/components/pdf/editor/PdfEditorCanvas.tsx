/**
 * Reusable visual PDF editor canvas (issue #6).
 * Renders a page, hosts an SVG draft overlay, object list, zoom/page chrome.
 * Does not write PDF output — that is issue #7+.
 *
 * MVP objects are rectangles only — enough to prove select/move/resize/undo and
 * coordinate accuracy. Text, images, freehand, and real PDF export land in #7–#8.
 */
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/Button";
import { Icon } from "@/components/ui/Icon";
import { Spinner } from "@/components/ui/Spinner";
import { Alert } from "@/components/ui/Alert";
import { pagePdf } from "@/lib/tauriCommands";
import { base64ToBytes } from "@/lib/pdfjs";
import type { EditDocument, PdfRect } from "@/lib/editor";
import { PageSurface, type PageLayout } from "./PageSurface";
import { EditorOverlay } from "./EditorOverlay";
import { ObjectList } from "./ObjectList";
import { useEditSession } from "./useEditSession";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 4;
const STEP = 0.25;

export function PdfEditorCanvas({
  sourcePath,
  pageNumber,
  pageCount,
  onPageChange,
  onChange,
}: {
  /** Absolute path to the PDF on disk. */
  sourcePath: string;
  /** 1-based page number (matches pagePdf). */
  pageNumber: number;
  pageCount: number;
  onPageChange?: (page: number) => void;
  onChange?: (doc: EditDocument) => void;
}) {
  const session = useEditSession(onChange);
  const [zoom, setZoom] = useState(1);
  const [bytes, setBytes] = useState<Uint8Array | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [layout, setLayout] = useState<PageLayout | null>(null);
  const [createMode, setCreateMode] = useState(true);
  const [surfaceKey, setSurfaceKey] = useState(0);
  /** Stable unzoomed fit width from the stage (not the page element). */
  const [fitWidth, setFitWidth] = useState(640);
  const stageRef = useRef<HTMLDivElement>(null);

  const pageIndex = pageNumber - 1;

  // Measure the stage once content can use it; zoom must not feed back into this.
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const measure = () => {
      const style = getComputedStyle(el);
      const pad =
        (parseFloat(style.paddingLeft) || 0) + (parseFloat(style.paddingRight) || 0);
      setFitWidth(Math.max(Math.floor(el.clientWidth - pad), 280));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [loading, bytes]);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setLoadError(null);
    setBytes(null);
    setLayout(null);
    pagePdf(sourcePath, pageNumber)
      .then((b64) => {
        if (!active) return;
        if (!b64) {
          setLoadError(
            "Could not extract this page for the editor. Check that qpdf is installed (brew install qpdf).",
          );
          setLoading(false);
          return;
        }
        const raw = base64ToBytes(b64);
        setBytes(raw.slice());
        setSurfaceKey((k) => k + 1);
        setLoading(false);
      })
      .catch((e: unknown) => {
        if (!active) return;
        const msg = e instanceof Error ? e.message : "Could not load this page.";
        setLoadError(`${msg} Is qpdf on PATH?`);
        setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [sourcePath, pageNumber]);

  const clampZoom = (z: number) =>
    Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(z * 100) / 100));

  const setZoomSafe = (next: number | ((z: number) => number)) => {
    setZoom((prev) => {
      const z = typeof next === "function" ? next(prev) : next;
      return clampZoom(z);
    });
  };

  const go = (delta: number) => {
    if (!onPageChange) return;
    const next = Math.min(pageCount, Math.max(1, pageNumber + delta));
    onPageChange(next);
  };

  const addCenteredRect = () => {
    if (!layout) return;
    const { box } = layout.geometry;
    const w = Math.min(120, box.w * 0.25);
    const h = Math.min(80, box.h * 0.15);
    const rect: PdfRect = {
      x: box.x + (box.w - w) / 2,
      y: box.y + (box.h - h) / 2,
      w,
      h,
    };
    session.addRect(pageIndex, rect);
  };

  return (
    <div className="pdf-editor">
      <div className="pdf-editor__toolbar thumb-toolbar wrap">
        <Button
          size="sm"
          variant="secondary"
          onClick={session.undo}
          disabled={!session.canUndo}
          leftIcon={<Icon name="undo" size={14} />}
        >
          Undo
        </Button>
        <Button size="sm" variant="secondary" onClick={session.redo} disabled={!session.canRedo}>
          Redo
        </Button>
        <span className="pdf-editor__sep" />
        <Button size="sm" variant="ghost" onClick={() => setZoomSafe((z) => z - STEP)}>
          −
        </Button>
        <span className="muted" style={{ fontSize: 12.5, minWidth: 40, textAlign: "center" }}>
          {Math.round(zoom * 100)}%
        </span>
        <Button size="sm" variant="ghost" onClick={() => setZoomSafe((z) => z + STEP)}>
          +
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setZoomSafe(1)}>
          Reset zoom
        </Button>
        <span className="pdf-editor__sep" />
        <Button size="sm" variant="ghost" onClick={() => go(-1)} disabled={pageNumber <= 1}>
          ← Page
        </Button>
        <span className="muted" style={{ fontSize: 12.5 }}>
          {pageNumber} / {pageCount}
        </span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => go(1)}
          disabled={pageNumber >= pageCount}
        >
          Page →
        </Button>
        <span className="pdf-editor__sep" />
        <Button
          size="sm"
          variant={createMode ? "primary" : "secondary"}
          onClick={() => setCreateMode((v) => !v)}
        >
          {createMode ? "Drag to draw" : "Select mode"}
        </Button>
        <Button size="sm" variant="secondary" onClick={addCenteredRect} disabled={!layout}>
          Add rectangle
        </Button>
      </div>

      <div className="pdf-editor__body">
        <aside className="pdf-editor__sidebar">
          <div className="pdf-editor__sidebar-title">Objects</div>
          <ObjectList
            objects={session.objects}
            selectedIds={session.selectedIds}
            onSelect={session.select}
            onDelete={session.remove}
          />
          {session.selectedIds.length > 0 && (
            <div className="muted" style={{ fontSize: 11.5, marginTop: 8 }}>
              Delete / Backspace removes selection. Arrows nudge (Shift = 10pt).
            </div>
          )}
          {layout && session.selectedIds[0] && (
            <SelectedCoords
              objects={session.objects}
              selectedId={session.selectedIds[0]}
            />
          )}
        </aside>

        <div className="pdf-editor__stage" ref={stageRef}>
          {loading && (
            <div className="pdf-editor__status">
              <Spinner /> Loading page…
            </div>
          )}
          {loadError && <Alert variant="danger">{loadError}</Alert>}
          {bytes && !loadError && (
            <div
              className="pdf-editor__page-wrap"
              style={
                layout
                  ? { width: layout.cssWidth, height: layout.cssHeight }
                  : { width: fitWidth, minHeight: 200 }
              }
            >
              <PageSurface
                key={`${sourcePath}:${pageNumber}:${surfaceKey}`}
                bytes={bytes}
                zoom={zoom}
                fitWidth={fitWidth}
                pageIndex={pageIndex}
                onLayout={setLayout}
                onFail={(reason) =>
                  setLoadError(reason ?? "pdf.js could not render this page.")
                }
              />
              {layout && (
                <EditorOverlay
                  layout={layout}
                  objects={session.objects}
                  selectedIds={session.selectedIds}
                  pageIndex={pageIndex}
                  createMode={createMode}
                  onSelect={session.select}
                  onClearSelection={session.clearSelection}
                  onBeginGesture={session.beginGesture}
                  onEndGesture={session.endGesture}
                  onUpdateRect={session.updateRect}
                  onCreateRect={(rect) => session.addRect(pageIndex, rect)}
                />
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function SelectedCoords({
  objects,
  selectedId,
}: {
  objects: { id: string; rect: PdfRect }[];
  selectedId: string;
}) {
  const obj = objects.find((o) => o.id === selectedId);
  if (!obj) return null;
  const { x, y, w, h } = obj.rect;
  const fmt = (n: number) => n.toFixed(1);
  return (
    <div className="pdf-editor__coords mono" style={{ fontSize: 11, marginTop: 10 }}>
      <div className="muted">PDF pts (export)</div>
      <div>
        x={fmt(x)} y={fmt(y)}
      </div>
      <div>
        w={fmt(w)} h={fmt(h)}
      </div>
    </div>
  );
}

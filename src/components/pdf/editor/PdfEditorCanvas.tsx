/**
 * Reusable visual PDF editor canvas.
 * Renders a page, hosts an SVG draft overlay, object list, zoom/page chrome.
 */
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { Button } from "@/components/ui/Button";
import { Icon, type IconName } from "@/components/ui/Icon";
import { Spinner } from "@/components/ui/Spinner";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import { listPdfAnnots, pagePdf, pickImageFile, previewImage } from "@/lib/tauriCommands";
import { toAppError } from "@/lib/types";
import { base64ToBytes } from "@/lib/pdfjs";
import type { EditObject, FormField, ShapeStyle } from "@/lib/editor";
import type { ListedMarkup } from "@/lib/types";
import {
  cloneObject,
  isClosedShapeObject,
  makeMapping,
  offsetObject,
  pdfRectToViewport,
  placeImagePdfRect,
  rgbToHex,
  selectedIdsOnPage,
  stageJustify,
} from "@/lib/editor";
import { PageSurface, type PageLayout } from "./PageSurface";
import { EditorOverlay, type EditorTool } from "./EditorOverlay";
import { FormFieldsOverlay } from "./FormFieldsOverlay";
import { ObjectList } from "./ObjectList";
import { ObjectInspector, type ColorPickTarget } from "./ObjectInspector";
import { ShapePicker, SHAPE_TOOLS } from "./ShapePicker";
import type { EditSession } from "./useEditSession";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 4;
const STEP = 0.25;
const PASTE_NUDGE = 14;

function newObjectId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `obj-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

const MAIN_TOOLS: {
  id: EditorTool;
  label: string;
  icon: "mousePointer" | "hand" | "type" | "image" | "pencil" | "external";
}[] = [
  { id: "select", label: "Select", icon: "mousePointer" },
  { id: "hand", label: "Hand", icon: "hand" },
  { id: "text", label: "Text", icon: "type" },
  { id: "image", label: "Image", icon: "image" },
  { id: "ink", label: "Draw", icon: "pencil" },
  { id: "link", label: "Link", icon: "external" },
];

const MARKUP_TOOLS: { id: EditorTool; label: string; icon: IconName }[] = [
  { id: "note", label: "Note", icon: "badge" },
  { id: "highlight", label: "Highlight", icon: "sparkles" },
  { id: "underline", label: "Underline", icon: "type" },
  { id: "strikeout", label: "Strikeout", icon: "slash" },
  { id: "markupInk", label: "Ink annot", icon: "stamp" },
];

function isTextEntryTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || !!el.isContentEditable;
}

export function PdfEditorCanvas({
  sourcePath,
  sourcePage,
  pageIndex,
  pageCount,
  session,
  onPageChange,
  formFields = [],
  formValues = {},
  onFormChange,
}: {
  sourcePath: string;
  /** 1-based page number inside `sourcePath` (pagePdf). */
  sourcePage: number;
  /** 0-based index in the combined editor session. */
  pageIndex: number;
  pageCount: number;
  session: EditSession;
  onPageChange?: (pageIndex: number) => void;
  formFields?: FormField[];
  formValues?: Record<string, string>;
  onFormChange?: (name: string, value: string) => void;
}) {
  const { toast } = useToast();
  const [zoom, setZoom] = useState(1);
  const [bytes, setBytes] = useState<Uint8Array | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [layout, setLayout] = useState<PageLayout | null>(null);
  const [tool, setTool] = useState<EditorTool>("text");
  const [spacePan, setSpacePan] = useState(false);
  const [panning, setPanning] = useState(false);
  const [shapeOpen, setShapeOpen] = useState(false);
  const [lastShape, setLastShape] = useState<(typeof SHAPE_TOOLS)[number]["id"]>("rect");
  const [surfaceKey, setSurfaceKey] = useState(0);
  const [fitWidth, setFitWidth] = useState(640);
  const [editingTextId, setEditingTextId] = useState<string | null>(null);
  const [colorPick, setColorPick] = useState<ColorPickTarget | null>(null);
  const [markupAuthor, setMarkupAuthor] = useState("");
  const [leftovers, setLeftovers] = useState<ListedMarkup[]>([]);
  const [pickCursor, setPickCursor] = useState<{ x: number; y: number } | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const pageCanvasRef = useRef<HTMLCanvasElement>(null);
  const panRef = useRef<{ x: number; y: number; sl: number; st: number } | null>(null);
  const clipboardRef = useRef<EditObject[]>([]);
  const pasteGen = useRef(1);
  const [canPaste, setCanPaste] = useState(false);
  const lastShapeStyle = useRef<ShapeStyle>({
    fill: "none",
    stroke: "#111827",
    strokeWidth: 1.5,
    opacity: 1,
  });
  const pageObjects = useMemo(
    () => session.objects.filter((object) => object.pageIndex === pageIndex),
    [pageIndex, session.objects],
  );
  const pageSelectedIds = useMemo(
    () => selectedIdsOnPage(session.objects, session.selectedIds, pageIndex),
    [pageIndex, session.objects, session.selectedIds],
  );

  useEffect(() => {
    setEditingTextId(null);
    setColorPick(null);
    setPickCursor(null);
  }, [pageIndex]);

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
    pagePdf(sourcePath, sourcePage)
      .then((b64) => {
        if (!active) return;
        if (!b64) {
          setLoadError("Could not open this page.");
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
        setLoadError(msg);
        setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [sourcePath, sourcePage]);

  useEffect(() => {
    let active = true;
    listPdfAnnots(sourcePath)
      .then((items) => {
        if (active) setLeftovers(items);
      })
      .catch(() => {
        if (active) setLeftovers([]);
      });
    return () => {
      active = false;
    };
  }, [sourcePath]);

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
    const next = Math.min(pageCount - 1, Math.max(0, pageIndex + delta));
    onPageChange(next);
  };

  const placeImage = async (atCss?: { x: number; y: number }) => {
    if (!layout) return;
    try {
      const path = await pickImageFile();
      if (!path) return;
      const preview = await previewImage(path);
      const rect = placeImagePdfRect(
        layout.geometry,
        layout.cssWidth,
        layout.cssHeight,
        preview.width,
        preview.height,
        atCss,
      );
      session.addImage(pageIndex, rect, path, preview.dataUrl);
      setTool("select");
    } catch (e) {
      const err = toAppError(e);
      toast({ title: err.title, description: err.message, variant: "error" });
    }
  };

  const copySelection = useCallback(() => {
    const activeIds = new Set(selectedIdsOnPage(session.objects, session.selectedIds, pageIndex));
    const sel = session.objects.filter((object) => activeIds.has(object.id));
    if (sel.length === 0) return false;
    clipboardRef.current = sel.map(cloneObject);
    pasteGen.current = 1;
    setCanPaste(true);
    return true;
  }, [pageIndex, session.objects, session.selectedIds]);

  const pasteClipboard = useCallback(() => {
    if (clipboardRef.current.length === 0) return;
    const n = pasteGen.current++;
    const dx = PASTE_NUDGE * n;
    const dy = -PASTE_NUDGE * n;
    session.addMany(
      clipboardRef.current.map((o) => {
        const next = offsetObject(o, dx, dy);
        next.id = newObjectId();
        next.pageIndex = pageIndex;
        return next;
      }),
    );
  }, [session, pageIndex]);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (isTextEntryTarget(t)) return;
      if (!root.contains(t) && document.activeElement && !root.contains(document.activeElement)) {
        return;
      }

      const mod = e.metaKey || e.ctrlKey;
      if (!mod && e.key.toLowerCase() === "h") {
        e.preventDefault();
        setTool("hand");
        return;
      }
      if (mod && e.key.toLowerCase() === "z" && !e.shiftKey) {
        e.preventDefault();
        session.undo();
        return;
      }
      if (mod && (e.key.toLowerCase() === "y" || (e.key.toLowerCase() === "z" && e.shiftKey))) {
        e.preventDefault();
        session.redo();
        return;
      }
      if (mod && e.key.toLowerCase() === "c") {
        if (copySelection()) e.preventDefault();
        return;
      }
      if (mod && e.key.toLowerCase() === "v") {
        if (clipboardRef.current.length === 0) return;
        e.preventDefault();
        pasteClipboard();
        return;
      }
      if (mod && e.key.toLowerCase() === "d") {
        if (pageSelectedIds.length === 0) return;
        e.preventDefault();
        copySelection();
        pasteClipboard();
        return;
      }
      if (e.key === "Escape") {
        if (shapeOpen) {
          setShapeOpen(false);
          return;
        }
        if (colorPick) {
          setColorPick(null);
          setPickCursor(null);
          return;
        }
        session.clearSelection();
        setEditingTextId(null);
        return;
      }
      if (e.key === "Delete" || e.key === "Backspace") {
        if (pageSelectedIds.length === 0) return;
        e.preventDefault();
        session.remove(pageSelectedIds);
        return;
      }
      if (pageSelectedIds.length === 0) return;
      const step = e.shiftKey ? 10 : 1;
      if (e.key === "ArrowLeft") {
        e.preventDefault();
        session.nudgeSelected(-step, 0, pageIndex);
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        session.nudgeSelected(step, 0, pageIndex);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        session.nudgeSelected(0, step, pageIndex);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        session.nudgeSelected(0, -step, pageIndex);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [session, colorPick, copySelection, pageIndex, pageSelectedIds, pasteClipboard, shapeOpen]);

  useEffect(() => {
    const stopSpacePan = () => {
      setSpacePan(false);
      panRef.current = null;
      setPanning(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code !== "Space" && e.key !== " ") return;
      if (isTextEntryTarget(e.target) || isTextEntryTarget(document.activeElement)) return;
      const ae = document.activeElement;
      const root = rootRef.current;
      const stage = stageRef.current;
      // Only when the editor shell or the page stage is focused — not toolbar buttons.
      if (ae !== root && !(ae instanceof Node && !!stage?.contains(ae))) return;
      e.preventDefault();
      if (e.repeat) return;
      setSpacePan(true);
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.code !== "Space" && e.key !== " ") return;
      stopSpacePan();
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", stopSpacePan);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", stopSpacePan);
    };
  }, []);

  const selected = pageObjects.find((object) => object.id === pageSelectedIds[0]) ?? null;
  const panMode = !colorPick && (tool === "hand" || spacePan);

  const onStagePointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    rootRef.current?.focus();
    if (!panMode) return;
    if (e.button !== 0) return;
    const el = stageRef.current;
    if (!el) return;
    panRef.current = { x: e.clientX, y: e.clientY, sl: el.scrollLeft, st: el.scrollTop };
    setPanning(true);
    try {
      el.setPointerCapture(e.pointerId);
    } catch {
      /* capture is best-effort */
    }
    e.preventDefault();
  };

  const onStagePointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const el = stageRef.current;
    const drag = panRef.current;
    if (!el || !drag) return;
    el.scrollLeft = drag.sl - (e.clientX - drag.x);
    el.scrollTop = drag.st - (e.clientY - drag.y);
  };

  const onStagePointerUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!panRef.current) return;
    panRef.current = null;
    setPanning(false);
    const el = stageRef.current;
    if (el?.hasPointerCapture(e.pointerId)) {
      try {
        el.releasePointerCapture(e.pointerId);
      } catch {
        /* ignore */
      }
    }
  };

  const applyPickedColor = (hex: string) => {
    if (!selected || !colorPick) return;
    if (colorPick === "color") session.updateObject(selected.id, { color: hex } as Partial<EditObject>);
    else if (colorPick === "fill") session.updateObject(selected.id, { fill: hex } as Partial<EditObject>);
    else session.updateObject(selected.id, { stroke: hex } as Partial<EditObject>);
    setColorPick(null);
    setPickCursor(null);
  };

  const samplePageColor = (css: { x: number; y: number }) => {
    const canvas = pageCanvasRef.current;
    if (!canvas || canvas.width < 1 || canvas.height < 1) return;
    const scaleX = canvas.width / Math.max(canvas.clientWidth, 1);
    const scaleY = canvas.height / Math.max(canvas.clientHeight, 1);
    const x = Math.min(canvas.width - 1, Math.max(0, Math.floor(css.x * scaleX)));
    const y = Math.min(canvas.height - 1, Math.max(0, Math.floor(css.y * scaleY)));
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;
    const data = ctx.getImageData(x, y, 1, 1).data;
    applyPickedColor(rgbToHex(data[0], data[1], data[2]));
  };

  return (
    <div className="pdf-editor" ref={rootRef} tabIndex={0}>
      {tool === "redact" && (
        <div className="pdf-editor__redact-note">
          <Alert variant="info">
            Redaction permanently removes content on Save. Only pages with a
            redaction region become images; text on those pages will not stay
            selectable.
          </Alert>
        </div>
      )}
      <div className="pdf-editor__toolbar thumb-toolbar wrap">
        <Button size="sm" variant="secondary" onClick={session.undo} disabled={!session.canUndo} title="Undo" aria-label="Undo">
          <Icon name="undo" size={15} />
        </Button>
        <Button size="sm" variant="secondary" onClick={session.redo} disabled={!session.canRedo} title="Redo" aria-label="Redo">
          <Icon name="undo" size={15} style={{ transform: "scaleX(-1)" }} />
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={copySelection}
          disabled={pageSelectedIds.length === 0}
          title="Copy (Ctrl/Cmd+C)"
          aria-label="Copy"
        >
          <Icon name="copy" size={15} />
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={pasteClipboard}
          disabled={!canPaste}
          title="Paste (Ctrl/Cmd+V)"
          aria-label="Paste"
        >
          <Icon name="clipboard" size={15} />
        </Button>
        <span className="pdf-editor__sep" />
        {MAIN_TOOLS.filter((t) => t.id !== "ink" && t.id !== "link").map((t) => (
          <Button
            key={t.id}
            size="sm"
            variant={tool === t.id ? "primary" : "ghost"}
            title={t.id === "hand" ? "Hand — drag to slide the page (H, hold Space)" : t.label}
            aria-label={t.id === "hand" ? "Hand" : t.label}
            aria-pressed={tool === t.id}
            onClick={() => {
              if (t.id === "image") {
                void placeImage();
                return;
              }
              setTool(t.id);
            }}
          >
            <Icon name={t.icon} size={16} />
          </Button>
        ))}
        <ShapePicker
          tool={tool}
          open={shapeOpen}
          lastShape={lastShape}
          onOpenChange={setShapeOpen}
          onPick={(id) => {
            setLastShape(id);
            setTool(id);
          }}
        />
        <Button
          size="sm"
          variant={tool === "ink" ? "primary" : "ghost"}
          title="Draw"
          aria-label="Draw"
          aria-pressed={tool === "ink"}
          onClick={() => setTool("ink")}
        >
          <Icon name="pencil" size={16} />
        </Button>
        <Button
          size="sm"
          variant={tool === "link" ? "primary" : "ghost"}
          title="Link — draw a hotspot (does not open the address)"
          aria-label="Link"
          aria-pressed={tool === "link"}
          onClick={() => setTool("link")}
        >
          <Icon name="external" size={16} />
        </Button>
        <Button
          size="sm"
          variant={tool === "redact" ? "primary" : "ghost"}
          title="Redaction — permanently remove content in this region on Save"
          aria-label="Redaction"
          aria-pressed={tool === "redact"}
          onClick={() => setTool("redact")}
        >
          <Icon name="squareFill" size={16} />
        </Button>
        {MARKUP_TOOLS.map((t) => (
          <Button
            key={t.id}
            size="sm"
            variant={tool === t.id ? "primary" : "ghost"}
            title={t.label}
            aria-label={t.label}
            aria-pressed={tool === t.id}
            onClick={() => setTool(t.id)}
          >
            <Icon name={t.icon} size={16} />
          </Button>
        ))}
        <span className="pdf-editor__sep" />
        <Button size="sm" variant="ghost" onClick={() => setZoomSafe((z) => z - STEP)} title="Zoom out" aria-label="Zoom out">
          <Icon name="minus" size={15} />
        </Button>
        <button type="button" className="btn btn--ghost btn--sm" title="Reset zoom" aria-label="Reset zoom" onClick={() => setZoomSafe(1)} style={{ minWidth: 44 }}>
          {Math.round(zoom * 100)}%
        </button>
        <Button size="sm" variant="ghost" onClick={() => setZoomSafe((z) => z + STEP)} title="Zoom in" aria-label="Zoom in">
          <Icon name="plus" size={15} />
        </Button>
        <span className="pdf-editor__sep" />
        <Button size="sm" variant="ghost" onClick={() => go(-1)} disabled={pageIndex <= 0} title="Previous page" aria-label="Previous page">
          <Icon name="chevronRight" size={15} style={{ transform: "rotate(180deg)" }} />
        </Button>
        <span className="muted" style={{ fontSize: 12.5 }}>
          {pageIndex + 1} / {pageCount}
        </span>
        <Button size="sm" variant="ghost" onClick={() => go(1)} disabled={pageIndex >= pageCount - 1} title="Next page" aria-label="Next page">
          <Icon name="chevronRight" size={15} />
        </Button>
      </div>

      <div className="pdf-editor__body">
        <aside className="pdf-editor__sidebar">
          <div className="pdf-editor__sidebar-title">Objects</div>
          <ObjectList
            objects={pageObjects}
            selectedIds={pageSelectedIds}
            onSelect={session.select}
            onDelete={session.remove}
          />
          {leftovers.filter((a) => a.pageIndex === sourcePage - 1).length > 0 && (
            <div className="pdf-editor__leftovers" style={{ marginTop: 12 }}>
              <div className="pdf-editor__sidebar-title">Existing annotations</div>
              <ul className="pdf-editor__object-list" aria-label="Existing annotations">
                {leftovers
                  .filter((a) => a.pageIndex === sourcePage - 1)
                  .map((a, i) => (
                    <li key={`${a.subtype}-${i}`} className="muted" style={{ fontSize: 12.5 }}>
                      {a.subtype}
                      {a.contents ? `: ${a.contents.slice(0, 24)}` : ""}
                      {a.author ? ` · ${a.author}` : ""}
                    </li>
                  ))}
              </ul>
            </div>
          )}
          <label className="field__label" style={{ marginTop: 10 }}>
            Annot author
          </label>
          <input
            className="input"
            type="text"
            value={markupAuthor}
            placeholder="Author"
            onChange={(e) => setMarkupAuthor(e.target.value)}
          />
          {selected && pageSelectedIds.length > 1 && (
            <div className="muted" style={{ fontSize: 12.5, marginTop: 10 }}>
              {pageSelectedIds.length} selected — drag to move together
            </div>
          )}
          {selected && pageSelectedIds.length === 1 && (
            <ObjectInspector
              obj={selected}
              picking={colorPick}
              pageCount={pageCount}
              layerIndex={pageObjects.findIndex((object) => object.id === selected.id) + 1}
              layerCount={pageObjects.length}
              onChange={(patch) => {
                session.updateObject(selected.id, patch);
                if (isClosedShapeObject(selected)) {
                  lastShapeStyle.current = { ...lastShapeStyle.current, ...patch };
                }
              }}
              onPickFromPage={(target) => {
                setColorPick((cur) => (cur === target ? null : target));
                setPickCursor(null);
              }}
              onReorder={(dir) => session.reorder(selected.id, dir)}
            />
          )}
        </aside>

        <div
          className={`pdf-editor__stage${colorPick ? " is-eyedrop" : ""}${panMode ? " is-hand" : ""}${panning ? " is-panning" : ""}${stageJustify(layout?.cssWidth ?? 0, fitWidth) === "start" ? " is-start" : ""}`}
          ref={stageRef}
          onPointerDown={onStagePointerDown}
          onPointerMove={onStagePointerMove}
          onPointerUp={onStagePointerUp}
          onPointerCancel={onStagePointerUp}
        >
          {loading && (
            <div className="pdf-editor__status">
              <Spinner /> Loading page…
            </div>
          )}
          {loadError && <Alert variant="danger">{loadError}</Alert>}
          {colorPick && (
            <div className="muted" style={{ fontSize: 12.5, padding: "8px 12px 0" }}>
              Click the page to sample a color · Esc cancels
            </div>
          )}
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
                key={`${sourcePath}:${sourcePage}:${surfaceKey}`}
                bytes={bytes}
                zoom={zoom}
                fitWidth={fitWidth}
                pageIndex={pageIndex}
                canvasRef={pageCanvasRef}
                onLayout={setLayout}
                onFail={(reason) => setLoadError(reason ?? "Could not render this page.")}
              />
              {layout && onFormChange && (
                <FormFieldsOverlay
                  layout={layout}
                  fields={formFields}
                  values={formValues}
                  sourcePage={sourcePage}
                  onChange={onFormChange}
                />
              )}
              {layout && (
                <EditorOverlay
                  layout={layout}
                  objects={pageObjects}
                  selectedIds={pageSelectedIds}
                  pageIndex={pageIndex}
                  tool={panMode ? "hand" : tool}
                  createStyle={lastShapeStyle.current}
                  pickColor={!!colorPick}
                  onPickColor={samplePageColor}
                  onPickHover={colorPick ? (p) => setPickCursor(p) : undefined}
                  onSelect={session.select}
                  onClearSelection={session.clearSelection}
                  onBeginGesture={session.beginGesture}
                  onEndGesture={session.endGesture}
                  onUpdateRect={session.updateRect}
                  onUpdateRotate={(id, deg) => session.updateObject(id, { objectRotate: deg })}
                  onCreateShape={(kind, rect, keepAspect) => {
                    session.addShape(kind, pageIndex, rect, lastShapeStyle.current, keepAspect);
                    setTool("select");
                  }}
                  onCreateText={(rect) => {
                    session.addText(pageIndex, rect);
                    setTool("select");
                  }}
                  onCreateLink={(rect) => {
                    session.addLink(pageIndex, rect);
                    setTool("select");
                  }}
                  onCreateLine={(a, b) => {
                    session.addLine(pageIndex, a.x, a.y, b.x, b.y);
                    setLastShape("line");
                    setTool("select");
                  }}
                  onCreateInk={(pts) => {
                    session.addInk(pageIndex, pts);
                    setTool("select");
                  }}
                  onCreateNote={(rect) => {
                    session.addNote(pageIndex, rect, markupAuthor);
                    setTool("select");
                  }}
                  onCreateHighlight={(rect) => {
                    session.addHighlight(pageIndex, rect, markupAuthor);
                    setTool("select");
                  }}
                  onCreateUnderline={(rect) => {
                    session.addUnderline(pageIndex, rect, markupAuthor);
                    setTool("select");
                  }}
                  onCreateStrikeout={(rect) => {
                    session.addStrikeout(pageIndex, rect, markupAuthor);
                    setTool("select");
                  }}
                  onCreateMarkupInk={(strokes) => {
                    session.addMarkupInk(pageIndex, strokes, markupAuthor);
                    setTool("select");
                  }}
                  onCreateRedact={(rect) => {
                    session.addRedact(pageIndex, rect);
                    setTool("select");
                  }}
                  onRequestImage={(at) => void placeImage(at)}
                  onActivateText={(id) => setEditingTextId(id)}
                />
              )}
              {editingTextId && layout && (
                <TextEditor
                  obj={pageObjects.find((object) => object.id === editingTextId)}
                  layout={layout}
                  onChange={(content) => session.updateObject(editingTextId, { content } as Partial<EditObject>)}
                  onClose={() => setEditingTextId(null)}
                />
              )}
            </div>
          )}
        </div>
      </div>
      {colorPick && pickCursor && (
        <div className="pdf-editor__pick-cursor" style={{ left: pickCursor.x, top: pickCursor.y }} aria-hidden>
          <Icon name="eyedropper" size={20} />
        </div>
      )}
    </div>
  );
}

function TextEditor({
  obj,
  layout,
  onChange,
  onClose,
}: {
  obj: EditObject | undefined;
  layout: PageLayout;
  onChange: (content: string) => void;
  onClose: () => void;
}) {
  if (!obj || obj.kind !== "text") return null;
  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);
  const css = pdfRectToViewport(obj.rect, mapping);
  const rot = obj.objectRotate ?? 0;
  return (
    <textarea
      className="pdf-editor__text-edit"
      style={{
        left: css.x,
        top: css.y,
        width: Math.max(css.w, 80),
        height: Math.max(css.h, 28),
        transform: rot ? `rotate(${rot}deg)` : undefined,
        transformOrigin: "center center",
      }}
      value={obj.content}
      autoFocus
      onChange={(e) => onChange(e.target.value)}
      onBlur={onClose}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
    />
  );
}

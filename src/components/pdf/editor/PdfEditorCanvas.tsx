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
import { Icon } from "@/components/ui/Icon";
import { Spinner } from "@/components/ui/Spinner";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import { listPdfAnnots, pagePdf, pickImageFile, previewImage } from "@/lib/tauriCommands";
import { toAppError } from "@/lib/types";
import { base64ToBytes } from "@/lib/pdfjs";
import type { EditObject, FormField, ShapeStyle } from "@/lib/editor";
import type { ListedMarkup } from "@/lib/types";
import type { TextStamp } from "@/lib/editor/sourceText";
import { UI } from "@/lib/editor/sourceTextCopy";
import type { TextSources } from "@/features/edit-pdf/useTextSources";
import {
  cloneObject,
  isClosedShapeObject,
  makeMapping,
  offsetObject,
  pdfRectToViewport,
  placeImagePdfRect,
  editsSignature,
  rgbToHex,
  selectedIdsOnPage,
  stageJustify,
} from "@/lib/editor";
import { PageSurface, type PageLayout } from "./PageSurface";
import { EditorOverlay, type EditorTool } from "./EditorOverlay";
import { FormFieldsOverlay } from "./FormFieldsOverlay";
import { ObjectList } from "./ObjectList";
import { ObjectInspector, type ColorPickTarget } from "./ObjectInspector";
import type { SHAPE_TOOLS } from "./ShapePicker";
import type { EditSession } from "./useEditSession";
import { DEFAULT_EDITOR_TOOL, EditorToolbar, editorShortcut } from "./EditorToolbar";
import {
  PreviewStatusChip,
  SourceTextBanners,
  SourceTextMode,
  useSourceTextPage,
  type SourceTextGuard,
  type TextRequest,
} from "./sourceText/SourceTextMode";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 4;
const STEP = 0.25;
const PASTE_NUDGE = 14;

function newObjectId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `obj-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

/** Inside the Edit text layer, inline editor, format bar or reason popover. */
function inSourceText(t: EventTarget | null): boolean {
  return t instanceof Element && !!t.closest("[data-source-text]");
}

function isTextEntryTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || !!el.isContentEditable;
}

/** What the canvas needs for Edit text (owned by `EditPdfPage`). */
export interface CanvasTextProps {
  sources: TextSources;
  /** Display name of `sourcePath`. */
  fileName: string;
  /** The shown source page is listed more than once. */
  duplicatePage: boolean;
  guardRef: { current: SourceTextGuard | null }; // the open line edit, so Save can finish it first
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
  text,
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
  text: CanvasTextProps;
}) {
  const { toast } = useToast();
  const [zoom, setZoom] = useState(1);
  const [bytes, setBytes] = useState<Uint8Array | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [layout, setLayout] = useState<PageLayout | null>(null);
  const [tool, setTool] = useState<EditorTool>(DEFAULT_EDITOR_TOOL);
  /** Show original is on for one set of changes; a commit, restore or undo turns it off. */
  const [originalFor, setOriginalFor] = useState<string | null>(null);
  const [modeDismissed, setModeDismissed] = useState(false);
  const [textRequest, setTextRequest] = useState<TextRequest | null>(null);
  const textGuard = text.guardRef;
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
  const st = useSourceTextPage({
    objects: session.objects,
    path: sourcePath,
    fileName: text.fileName,
    sourcePage,
    pageIndex,
    sources: text.sources,
    active: tool === "editText",
    duplicate: text.duplicatePage,
  });
  const hasTextChanges = st.objects.length > 0;
  const textSig = editsSignature(st.objects);
  const original = st.canShowOriginal && originalFor === textSig;
  const toggleOriginal = () => setOriginalFor((at) => (at === textSig ? null : textSig));
  const surfaceBytes = bytes && (original || !st.preview.bytes ? bytes : st.preview.bytes);

  useEffect(() => {
    setEditingTextId(null);
    setColorPick(null);
    setPickCursor(null);
    setOriginalFor(null);
    setTextRequest(null);
  }, [pageIndex]);

  /** An open line edit must finish (or stay, with its message) before the page or tool changes. */
  const finishTextEdit = async (): Promise<boolean> => {
    const guard = textGuard.current;
    if (!guard?.isEditing() || (await guard.tryClose())) return true;
    toast({ title: UI.navBlocked, variant: "error" });
    return false;
  };

  const requestTool = async (next: EditorTool) => {
    if (next !== tool && (await finishTextEdit())) setTool(next);
  };

  /** Switch to Edit text and focus (or open) one line, from the sidebar or inspector. */
  const showTextLine = (runId: string, open: boolean) => {
    setTool("editText");
    setTextRequest((r) => ({ runId, open, tick: (r?.tick ?? 0) + 1 }));
  };

  /** A drawn object is placed: back to Select. */
  const thenSelect = <A extends unknown[]>(place: (...args: A) => void) => (...args: A) => {
    place(...args);
    setTool("select");
  };

  const addTextHere = (stamp: TextStamp) => {
    const id = session.addTextStamp(pageIndex, stamp.rect, stamp.fontSize);
    session.select([id]);
    setTool("text");
    setEditingTextId(id);
  };

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

  const go = async (delta: number) => {
    if (!onPageChange) return;
    const next = Math.min(pageCount - 1, Math.max(0, pageIndex + delta));
    if (next !== pageIndex && (await finishTextEdit())) onPageChange(next);
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
    // Text changes are never copied (one per line, locked to it).
    const sel = session.objects.filter((object) => activeIds.has(object.id) && object.kind !== "sourceText");
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
      const shortcut = t?.closest?.(".st-editor, .st-popover") ? null : editorShortcut(e, st.canShowOriginal);
      if (shortcut) {
        e.preventDefault();
        if (shortcut.kind === "tool") void requestTool(shortcut.tool);
        else toggleOriginal();
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
        if (copySelection()) pasteClipboard();
        return;
      }
      // The Edit text layer owns arrows, Enter, Delete and Esc while it has focus.
      if (inSourceText(t) && /^(Arrow|Enter$|Escape$|Delete$|Backspace$|Home$|End$|Page)/.test(e.key)) return;
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
  }, [session, colorPick, copySelection, pageIndex, pageSelectedIds, pasteClipboard, shapeOpen, st.canShowOriginal, requestTool]);

  useEffect(() => {
    const stopSpacePan = () => {
      setSpacePan(false);
      panRef.current = null;
      setPanning(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code !== "Space" && e.key !== " ") return;
      if (isTextEntryTarget(e.target) || isTextEntryTarget(document.activeElement)) return;
      // Space activates a focused line or button of Edit text; it never pans there.
      if (inSourceText(e.target) || inSourceText(document.activeElement)) return;
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
  const layerObjects = pageObjects.filter((object) => object.kind !== "sourceText");
  const panMode = !colorPick && (tool === "hand" || spacePan);

  const onStagePointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    // The inline editor and the reason popover keep their own focus.
    if (!(e.target instanceof Element && e.target.closest(".st-editor, .st-popover"))) rootRef.current?.focus({ preventScroll: true });
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
            {hasTextChanges && ` ${UI.redactionConflict}`}
          </Alert>
        </div>
      )}
      <EditorToolbar
        tool={tool}
        onTool={(next) => void requestTool(next)}
        onImage={() => void finishTextEdit().then((ok) => (ok ? placeImage() : undefined))}
        shapeOpen={shapeOpen}
        onShapeOpenChange={setShapeOpen}
        lastShape={lastShape}
        onPickShape={(id) => {
          setLastShape(id);
          void requestTool(id);
        }}
        history={{
          canUndo: session.canUndo,
          canRedo: session.canRedo,
          onUndo: session.undo,
          onRedo: session.redo,
          canCopy: pageSelectedIds.length > 0,
          onCopy: () => void copySelection(),
          canPaste,
          onPaste: pasteClipboard,
        }}
        showOriginal={{ visible: st.canShowOriginal, pressed: original, onToggle: toggleOriginal }}
        zoom={zoom}
        onZoom={(step) => setZoomSafe(step === 0 ? 1 : (z) => z + step * STEP)}
        pageIndex={pageIndex}
        pageCount={pageCount}
        onPage={(delta) => void go(delta)}
      />

      <div className="pdf-editor__body">
        <aside className="pdf-editor__sidebar">
          <div className="pdf-editor__sidebar-title">Objects</div>
          <ObjectList
            objects={pageObjects}
            selectedIds={pageSelectedIds}
            onSelect={(ids) => {
              session.select(ids);
              const picked = ids.length === 1 ? pageObjects.find((o) => o.id === ids[0]) : undefined;
              if (picked?.kind === "sourceText") showTextLine(picked.runId, false);
            }}
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
              layerIndex={layerObjects.findIndex((object) => object.id === selected.id) + 1}
              layerCount={layerObjects.length}
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
              sourceText={
                selected.kind === "sourceText"
                  ? {
                      run: st.pageText.page?.runs.find((r) => r.id === selected.runId) ?? null,
                      fonts: st.fonts,
                      onEditLine: () => showTextLine(selected.runId, true),
                      onRestore: () => session.revertSourceText(selected.id),
                    }
                  : undefined
              }
            />
          )}
        </aside>

        <div className="pdf-editor__main">
          <SourceTextBanners
            page={st}
            onRemoveStale={session.removeSourceTextForFingerprint}
            modeDismissed={modeDismissed}
            onDismissMode={() => setModeDismissed(true)}
          />
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
                  bytes={surfaceBytes ?? bytes}
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
                    onCreateShape={thenSelect((kind, rect, keepAspect) => session.addShape(kind, pageIndex, rect, lastShapeStyle.current, keepAspect))}
                    onCreateText={thenSelect((rect) => session.addText(pageIndex, rect))}
                    onCreateLink={thenSelect((rect) => session.addLink(pageIndex, rect))}
                    onCreateLine={(a, b) => {
                      session.addLine(pageIndex, a.x, a.y, b.x, b.y);
                      setLastShape("line");
                      setTool("select");
                    }}
                    onCreateInk={thenSelect((pts) => session.addInk(pageIndex, pts))}
                    onCreateNote={thenSelect((rect) => session.addNote(pageIndex, rect, markupAuthor))}
                    onCreateHighlight={thenSelect((rect) => session.addHighlight(pageIndex, rect, markupAuthor))}
                    onCreateUnderline={thenSelect((rect) => session.addUnderline(pageIndex, rect, markupAuthor))}
                    onCreateStrikeout={thenSelect((rect) => session.addStrikeout(pageIndex, rect, markupAuthor))}
                    onCreateMarkupInk={thenSelect((strokes) => session.addMarkupInk(pageIndex, strokes, markupAuthor))}
                    onCreateRedact={thenSelect((rect) => session.addRedact(pageIndex, rect))}
                    onRequestImage={(at) => void placeImage(at)}
                    onActivateText={(id) => setEditingTextId(id)}
                  />
                )}
                {layout && tool === "editText" && (
                  <SourceTextMode
                    key={`${pageIndex}:${sourcePath}:${sourcePage}`}
                    page={st}
                    layout={layout}
                    session={session}
                    stage={stageRef.current}
                    inert={panMode}
                    guardRef={textGuard}
                    request={textRequest}
                    onRequestHandled={() => setTextRequest(null)}
                    onLeave={() => rootRef.current?.focus({ preventScroll: true })}
                    onAddTextHere={addTextHere}
                  />
                )}
                {layout && <PreviewStatusChip page={st} showOriginal={original} />}
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

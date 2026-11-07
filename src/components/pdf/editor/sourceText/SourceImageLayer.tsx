/**
 * Existing images of the shown page as movable boxes (issue #129).
 *
 * Every directly painted, movable image occurrence gets one box. Dragging one (or pressing
 * Alt+Arrow on a focused box) previews the page through the same planner and checks as Save and
 * commits exactly one image move, so one drag is one undo step. An image move only rewrites that
 * occurrence's `Do`: the image bytes and other occurrences of the same image stay as they are.
 * An occurrence the planner refuses is shown disabled with its reason instead of being guessed at.
 */
import {
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import {
  addMove,
  displayedSize,
  makeMapping,
  makeSourceImageObject,
  movedImageRect,
  pdfRectToViewport,
  toImageEditIn,
  toTextEditIn,
  viewportToPdf,
  type CssRect,
  type PdfRect,
  type SourceImageObject,
} from "@/lib/editor";
import { UI, fillCopy, problemCopy, reasonCopy } from "@/lib/editor/sourceTextCopy";
import { toAppError, type ImageEditIn, type ImageOccurrence } from "@/lib/types";
import type { PageLayout } from "../PageSurface";
import type { EditSession } from "../useEditSession";
import type { SourceTextPage } from "./SourceTextMode";

/** A click this soon after a key activation is the browser's echo of it. */
const KEY_ECHO_MS = 500;
const DRAG_START_PX = 3;

function newObjectId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `img-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

interface ImageView {
  occ: ImageOccurrence;
  obj: SourceImageObject | null;
  css: CssRect;
  state: "movable" | "refused" | "moved";
  label: string;
  describe: string;
}

export interface SourceImageLayerProps {
  page: SourceTextPage;
  layout: PageLayout;
  session: EditSession;
  /** Space-to-pan is active: let the stage take the pointer. */
  inert: boolean;
  /** Esc: focus goes back to the editor. */
  onLeave: () => void;
}

export function SourceImageLayer({ page, layout, session, inert, onLeave }: SourceImageLayerProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const busy = useRef(false);
  const keyEcho = useRef(0);
  const [drag, setDrag] = useState<{ locator: string; dx: number; dy: number } | null>(null);
  const [said, setSaid] = useState({ text: "", n: 0 });
  const dragRef = useRef<{
    locator: string;
    occ: ImageOccurrence;
    pointerId: number;
    start: { x: number; y: number };
    dx: number;
    dy: number;
    moved: boolean;
  } | null>(null);

  const announce = (sentence: string) => setSaid((s) => ({ text: sentence, n: s.n + 1 }));
  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);
  const pxPerPt = layout.cssWidth / Math.max(displayedSize(layout.geometry).w, 1);

  const views = useMemo<ImageView[]>(() => {
    const occurrences = page.pageText.page?.images ?? [];
    const byLocator = new Map(page.images.map((o) => [o.locator, o]));
    return occurrences.map((occ) => {
      const obj = byLocator.get(occ.locator) ?? null;
      const rect: PdfRect = obj?.rect ?? movedImageRect(occ.rect, undefined);
      const movable = occ.movable && !page.duplicate;
      const reason = occ.reason ? reasonCopy(occ.reason).body : null;
      return {
        occ,
        obj,
        css: pdfRectToViewport(rect, mapping),
        state: !movable ? "refused" : obj ? "moved" : "movable",
        label: fillCopy(UI.image.label, { n: page.pageIndex + 1 }),
        describe: movable
          ? UI.image.hint
          : fillCopy(UI.image.refused, { body: reason ?? reasonCopy(null).body }),
      };
    });
  }, [page.pageText.page, page.images, page.duplicate, page.pageIndex, mapping]);

  const local = (e: { clientX: number; clientY: number }) => {
    const r = rootRef.current?.getBoundingClientRect();
    return { x: e.clientX - (r?.left ?? 0), y: e.clientY - (r?.top ?? 0) };
  };

  /** A screen-space displacement as an unrotated PDF delta (works for /Rotate 0/90/180/270). */
  const screenDeltaToPdf = (dxPx: number, dyPx: number) => {
    const p0 = viewportToPdf({ x: 0, y: 0 }, mapping);
    const p1 = viewportToPdf({ x: dxPx, y: dyPx }, mapping);
    return { x: p1.x - p0.x, y: p1.y - p0.y };
  };

  /**
   * Plan the whole page with one image at a new displacement, then commit exactly one move only
   * when the same plan/qpdf/checks as Save accept it. A refused move leaves the page as it was.
   */
  const moveImage = async (occ: ImageOccurrence, delta: { x: number; y: number }) => {
    if (busy.current || inert || page.duplicate || !page.fingerprint || !occ.movable) return;
    const existing = page.images.find((o) => o.locator === occ.locator) ?? null;
    const nextMove = addMove(existing?.move, delta);
    if (!nextMove && !existing) return;
    const fingerprint = page.fingerprint;
    const candidate: ImageEditIn = { locator: occ.locator, move: nextMove ?? { x: 0, y: 0 } };
    const others = page.images.filter((o) => o.locator !== occ.locator).map(toImageEditIn);
    busy.current = true;
    try {
      const preview = await page.preview.request(
        page.objects.map(toTextEditIn),
        [...others, candidate],
      );
      const verdict = preview.verdicts.find((v) => v.runId === occ.locator);
      if (!verdict?.ok || preview.pageProblem) {
        announce(
          verdict && !verdict.ok
            ? verdict.code === "TEXT_EDIT_REFUSED" && verdict.reason
              ? fillCopy(UI.image.refused, { body: reasonCopy(verdict.reason).body })
              : problemCopy(verdict.code ?? "EDIT_VERIFY_FAILED", [], { name: page.fileName })
            : preview.pageProblem
              ? problemCopy(preview.pageProblem.code, [], { name: page.fileName })
              : fillCopy(UI.image.refused, { body: reasonCopy(occ.reason).body }),
        );
        return;
      }
      if (!nextMove) {
        if (existing) session.revertSourceImage(existing.id);
        announce(UI.image.movedHome);
        return;
      }
      const rect = verdict.newRect ?? movedImageRect(occ.rect, nextMove);
      session.setSourceImage(
        makeSourceImageObject(existing?.id ?? newObjectId(), page.pageIndex, rect, {
          locator: occ.locator,
          sourceFingerprint: fingerprint,
          sourcePageIndex: page.sourcePageIndex,
          move: nextMove,
        }),
      );
      announce(fillCopy(UI.image.moved, { dx: delta.x.toFixed(1), dy: delta.y.toFixed(1) }));
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "STALE" || err.code === "PDF_NEEDS_REPAIR") page.reportFault(err);
      else announce(err.message);
    } finally {
      busy.current = false;
    }
  };

  const startDrag = (e: PointerEvent<HTMLButtonElement>, v: ImageView) => {
    if (inert || page.duplicate || !v.occ.movable || busy.current || e.button !== 0) return;
    dragRef.current = {
      locator: v.occ.locator,
      occ: v.occ,
      pointerId: e.pointerId,
      start: local(e),
      dx: 0,
      dy: 0,
      moved: false,
    };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };

  const moveDrag = (e: PointerEvent<HTMLButtonElement>) => {
    const d = dragRef.current;
    if (!d || d.pointerId !== e.pointerId) return;
    const p = local(e);
    const dx = p.x - d.start.x;
    const dy = p.y - d.start.y;
    if (!d.moved && Math.hypot(dx, dy) < DRAG_START_PX) return;
    d.moved = true;
    d.dx = dx;
    d.dy = dy;
    setDrag({ locator: d.locator, dx, dy });
  };

  const endDrag = (e: PointerEvent<HTMLButtonElement>, cancelled: boolean) => {
    const d = dragRef.current;
    if (!d || d.pointerId !== e.pointerId) return;
    dragRef.current = null;
    if (e.currentTarget.hasPointerCapture?.(d.pointerId)) {
      e.currentTarget.releasePointerCapture(d.pointerId);
    }
    if (!d.moved) {
      setDrag(null);
      return;
    }
    // A drag is not a click, even when it is too small to be a move.
    keyEcho.current = Date.now();
    if (cancelled) {
      setDrag(null);
      return;
    }
    const p0 = viewportToPdf(d.start, mapping);
    const p1 = viewportToPdf({ x: d.start.x + d.dx, y: d.start.y + d.dy }, mapping);
    const delta = { x: p1.x - p0.x, y: p1.y - p0.y };
    if (Math.hypot(delta.x, delta.y) < 0.01) {
      setDrag(null);
      return;
    }
    void moveImage(d.occ, delta).finally(() => setDrag(null));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>, v: ImageView) => {
    if (e.altKey && e.key.startsWith("Arrow")) {
      e.preventDefault();
      e.stopPropagation();
      if (!v.occ.movable) {
        announce(v.describe);
        return;
      }
      const step = (e.shiftKey ? 10 : 1) * pxPerPt;
      const delta = screenDeltaToPdf(
        e.key === "ArrowRight" ? step : e.key === "ArrowLeft" ? -step : 0,
        e.key === "ArrowDown" ? step : e.key === "ArrowUp" ? -step : 0,
      );
      if (Math.hypot(delta.x, delta.y) >= 0.01) void moveImage(v.occ, delta);
      return;
    }
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      e.stopPropagation();
      if (v.obj) {
        session.revertSourceImage(v.obj.id);
        announce(UI.image.movedHome);
      }
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onLeave();
    }
  };

  if (views.length === 0) return null;

  return (
    <div
      ref={rootRef}
      className={`si-layer${inert ? " is-inert" : ""}`}
      role="group"
      aria-label={fillCopy(UI.image.layer, { n: page.pageIndex + 1 })}
      data-source-image=""
      style={!inert ? { touchAction: "none" } : undefined}
    >
      {views.map((v) => {
        const pending = drag?.locator === v.occ.locator ? drag : null;
        const refused = v.state === "refused";
        return (
          <button
            key={v.occ.locator}
            type="button"
            className={`si-image is-${v.state}${pending ? " is-dragging" : ""}`}
            style={{
              left: v.css.x,
              top: v.css.y,
              width: Math.max(v.css.w, 1),
              height: Math.max(v.css.h, 1),
              ...(pending ? { transform: `translate(${pending.dx}px, ${pending.dy}px)` } : {}),
            }}
            aria-label={v.label}
            aria-disabled={refused ? true : undefined}
            aria-describedby={`si-${v.occ.locator}`}
            aria-keyshortcuts={
              refused ? undefined : "Alt+ArrowLeft Alt+ArrowRight Alt+ArrowUp Alt+ArrowDown"
            }
            onPointerDown={(e) => startDrag(e, v)}
            onPointerMove={moveDrag}
            onPointerUp={(e) => endDrag(e, false)}
            onPointerCancel={(e) => endDrag(e, true)}
            onLostPointerCapture={(e) => {
              if (dragRef.current?.pointerId === e.pointerId) {
                dragRef.current = null;
                setDrag(null);
              }
            }}
            onKeyDown={(e) => onKeyDown(e, v)}
            onClick={(e) => {
              e.stopPropagation();
              if (Date.now() - keyEcho.current <= KEY_ECHO_MS) return;
              announce(refused ? v.describe : UI.image.hint);
            }}
          >
            <span className="si-image__tag" aria-hidden>
              {v.state === "moved" ? "\u21bb" : ""}
            </span>
          </button>
        );
      })}
      <div hidden>
        {views.map((v) => (
          <span key={v.occ.locator} id={`si-${v.occ.locator}`}>
            {v.describe}
          </span>
        ))}
      </div>
      <div className="si-hint">{UI.image.hint}</div>
      <div className="sr-only" role="status" aria-live="polite">
        {said.text}
        {said.n % 2 === 1 ? "\u200b" : ""}
      </div>
    </div>
  );
}

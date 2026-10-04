/**
 * The text lines of one page as real buttons in reading order (SPEC §D.5,
 * §D.6, §D.9, §D.10): one Tab stop with roving focus, arrows move through
 * `readingNeighbour`, Enter/F2/Space edit or explain, Delete restores, Esc
 * leaves. The pointer is hit-tested by the layer itself (smallest box, then
 * nearest baseline; ≥ 24 px tall hit areas), edited lines with their new
 * geometry. Only lines near the visible part of the stage are rendered, plus
 * the focused, open and edited ones.
 */
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { Icon } from "@/components/ui/Icon";
import {
  caretIndexAt,
  editedGeometry,
  makeMapping,
  pdfRectToViewport,
  pdfToViewport,
  problemMessage,
  readingNeighbour,
  viewportToPdf,
  type CssRect,
  type NeighbourKey,
  type SourceTextObject,
} from "@/lib/editor";
import { UI, editedRunLabel, fillCopy, reasonCopy, warningCopy } from "@/lib/editor/sourceTextCopy";
import type { TextFont, TextPreview, TextRun } from "@/lib/types";
import type { PageLayout } from "../PageSurface";

const MIN_HIT_PX = 24;
const VIEW_MARGIN_PX = 200;
const TOOLTIP_DELAY_MS = 400;
/** A click that arrives this soon after a key activation is the browser's echo of it. */
const KEY_ECHO_MS = 500;

const KEYS: Record<string, NeighbourKey> = {
  ArrowDown: "down",
  ArrowUp: "up",
  ArrowRight: "right",
  ArrowLeft: "left",
  Home: "home",
  End: "end",
  PageUp: "pageUp",
  PageDown: "pageDown",
};

interface RunView {
  run: TextRun;
  obj: SourceTextObject | null;
  css: CssRect;
  hit: CssRect;
  area: number;
  caretOffsets: number[];
  origin: { x: number; y: number };
  dir: { x: number; y: number };
  label: string;
  describe: string;
  state: "editable" | "refused" | "edited";
  flagged: "warning" | "problem" | null;
}

export interface SourceTextLayerProps {
  runs: TextRun[];
  objects: SourceTextObject[];
  fonts: Map<string, TextFont>;
  layout: PageLayout;
  /** 1-based page number for the layer's label. */
  pageNumber: number;
  /** The preview answer for the page's current changes, if any. */
  preview: TextPreview | null;
  /** The page is listed twice: everything is shown refused and never edited. */
  duplicate: boolean;
  focusedId: string | null;
  /** Bumped to move DOM focus to `focusedId`. */
  focusTick: number;
  editingId: string | null;
  popoverId: string | null;
  /** Scroll container, for rendering only what is near view. */
  stage: HTMLElement | null;
  /** Space-to-pan is active: let the stage take the pointer. */
  inert: boolean;
  onFocusedChange: (id: string, moveFocus: boolean) => void;
  onEdit: (run: TextRun, caret: number | "all") => void;
  onExplain: (run: TextRun) => void;
  onRestore: (obj: SourceTextObject) => void;
  onLeave: () => void;
}

/** Text of the next run on the same line, for the overlap message. */
export function nextOnLine(runs: TextRun[], run: TextRun): TextRun | null {
  let best: TextRun | null = null;
  for (const r of runs) if (r.line === run.line && r.order > run.order && (!best || r.order < best.order)) best = r;
  return best;
}

function contains(r: CssRect, p: { x: number; y: number }): boolean {
  return p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h;
}

function intersects(a: CssRect, b: CssRect): boolean {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
}

function hitTest(views: RunView[], p: { x: number; y: number }): RunView | null {
  let best: RunView | null = null;
  let bestDist = Infinity;
  for (const v of views) {
    if (!contains(v.hit, p)) continue;
    const dist = Math.abs((p.x - v.origin.x) * v.dir.y - (p.y - v.origin.y) * v.dir.x);
    if (!best || v.area < best.area - 0.5 || (Math.abs(v.area - best.area) <= 0.5 && dist < bestDist)) {
      best = v;
      bestDist = dist;
    }
  }
  return best;
}

type ViewInputs = Pick<SourceTextLayerProps, "runs" | "objects" | "fonts" | "layout" | "preview" | "duplicate">;

function buildViews({ runs, objects, fonts, layout, preview, duplicate }: ViewInputs): RunView[] {
  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);
  const byRun = new Map(objects.map((o) => [o.runId, o]));
  const verdicts = new Map((preview?.verdicts ?? []).map((v) => [v.runId, v]));
  return runs
    .slice()
    .sort((a, b) => a.order - b.order)
    .map((run) => {
      const obj = byRun.get(run.id) ?? null;
      const verdict = verdicts.get(run.id) ?? null;
      const geom = obj ? editedGeometry(run, obj, verdict, fonts) : { rect: run.rect, caretOffsets: run.caretOffsets };
      const css = pdfRectToViewport(geom.rect, mapping);
      const h = Math.max(css.h, MIN_HIT_PX);
      const o = pdfToViewport(run.origin, mapping);
      const o2 = pdfToViewport({ x: run.origin.x + run.dir.x, y: run.origin.y + run.dir.y }, mapping);
      const len = Math.hypot(o2.x - o.x, o2.y - o.y) || 1;
      const notes: string[] = [];
      let flagged: RunView["flagged"] = null;
      if (verdict && !verdict.ok && obj) {
        notes.push(problemMessage(verdict, run, fonts));
        flagged = "problem";
      }
      for (const w of preview?.warnings ?? []) {
        if (w.runId !== run.id) continue;
        notes.push(warningCopy(w.code, nextOnLine(runs, run)?.text));
        flagged = flagged ?? "warning";
      }
      if (run.substituted) notes.push(UI.substituted);
      const refused = duplicate || !run.editable;
      if (duplicate) notes.unshift(UI.banner.duplicatePage);
      else if (!run.editable) notes.unshift(fillCopy(UI.run.refusedDescribe, { short: reasonCopy(run.reason).short }));
      return {
        run,
        obj,
        css,
        hit: { x: css.x, y: css.y + css.h / 2 - h / 2, w: css.w, h },
        area: css.w * css.h,
        caretOffsets: geom.caretOffsets,
        origin: o,
        dir: { x: (o2.x - o.x) / len, y: (o2.y - o.y) / len },
        label: refused
          ? run.text
          : obj
            ? editedRunLabel(obj.text, obj.originalText)
            : fillCopy(UI.run.editable, { text: run.text }),
        describe: notes.join(" "),
        state: refused ? "refused" : obj ? "edited" : "editable",
        flagged,
      };
    });
}

export function SourceTextLayer(props: SourceTextLayerProps) {
  const { layout, duplicate, focusedId, focusTick, stage } = props;
  const uid = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const keyEcho = useRef(0);
  const [hoverId, setHoverId] = useState<string | null>(null);
  const [tipId, setTipId] = useState<string | null>(null);
  const [view, setView] = useState<CssRect | null>(null);

  const { runs, objects, fonts, preview } = props;
  const views = useMemo(
    () => buildViews({ runs, objects, fonts, layout, preview, duplicate }),
    [runs, objects, fonts, layout, preview, duplicate],
  );
  const rovingId = views.some((v) => v.run.id === focusedId) ? focusedId : (views[0]?.run.id ?? null);

  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!stage || !root) {
      setView(null);
      return;
    }
    let frame = 0;
    const measure = () => {
      frame = 0;
      const s = stage.getBoundingClientRect();
      const l = root.getBoundingClientRect();
      const m = VIEW_MARGIN_PX;
      setView({ x: s.left - l.left - m, y: s.top - l.top - m, w: s.width + 2 * m, h: s.height + 2 * m });
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(measure);
    };
    measure();
    stage.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    return () => {
      stage.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [stage, layout]);

  // Move DOM focus only when asked (`focusTick`), never because the roving line changed.
  const focusedRef = useRef(focusedId);
  focusedRef.current = focusedId;
  useEffect(() => {
    const id = focusedRef.current;
    if (!focusTick || !id) return;
    const el = buttons.current.get(id);
    el?.focus();
    el?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [focusTick]);

  const tipText = (v: RunView | null): string | null => {
    if (!v || duplicate) return null;
    if (v.state === "refused") return fillCopy(UI.run.refusedDescribe, { short: reasonCopy(v.run.reason).short });
    return v.run.substituted ? UI.substituted : null;
  };
  const hovered = views.find((v) => v.run.id === hoverId) ?? null;
  const hoveredTip = tipText(hovered);
  useEffect(() => {
    setTipId(null);
    if (!hoveredTip || !hoverId) return;
    const timer = setTimeout(() => setTipId(hoverId), TOOLTIP_DELAY_MS);
    return () => clearTimeout(timer);
  }, [hoverId, hoveredTip]);

  const pinned = new Set([rovingId, props.editingId, props.popoverId, ...objects.map((o) => o.runId)]);
  const shown = views.filter((v) => pinned.has(v.run.id) || !view || intersects(v.hit, view));

  const activate = (v: RunView, caret: number | "all") => {
    props.onFocusedChange(v.run.id, false);
    if (duplicate) return;
    if (v.run.editable) props.onEdit(v.run, caret);
    else props.onExplain(v.run);
  };

  const local = (e: { clientX: number; clientY: number }) => {
    const r = rootRef.current?.getBoundingClientRect();
    return { x: e.clientX - (r?.left ?? 0), y: e.clientY - (r?.top ?? 0) };
  };

  const onRunKeyDown = (e: KeyboardEvent<HTMLButtonElement>, v: RunView) => {
    const move = KEYS[e.key];
    if (move) {
      e.preventDefault();
      e.stopPropagation();
      const next = readingNeighbour(runs, v.run.id, move);
      if (next) props.onFocusedChange(next, true);
    } else if (e.key === "Enter" || e.key === "F2" || e.key === " ") {
      e.preventDefault();
      e.stopPropagation();
      keyEcho.current = Date.now();
      activate(v, "all");
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      e.stopPropagation();
      if (v.obj) props.onRestore(v.obj);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      props.onLeave();
    }
  };

  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);
  // The reason dialog opens where the tooltip sits (just under the line) and says more: no tooltip under it.
  const tip = props.popoverId ? null : (views.find((v) => v.run.id === tipId) ?? null);
  const cursor = hovered ? (hovered.state === "refused" ? " is-over-refused" : " is-over-editable") : "";

  return (
    <div
      ref={rootRef}
      className={`st-layer${props.inert ? " is-inert" : ""}${cursor}`}
      role="group"
      aria-label={fillCopy(UI.layer.label, { n: props.pageNumber })}
      data-source-text=""
      onPointerMove={(e: PointerEvent<HTMLDivElement>) => setHoverId(hitTest(views, local(e))?.run.id ?? null)}
      onPointerLeave={() => setHoverId(null)}
      onClick={(e) => {
        if (e.target !== e.currentTarget || e.button !== 0) return;
        const p = local(e);
        const v = hitTest(views, p);
        if (v) activate(v, v.run.editable ? caretIndexAt(v.run, viewportToPdf(p, mapping), v.caretOffsets) : "all");
      }}
    >
      {shown.map((v, i) => (
        <button
          key={v.run.id}
          ref={(el) => {
            if (el) buttons.current.set(v.run.id, el);
            else buttons.current.delete(v.run.id);
          }}
          type="button"
          className={`st-run is-${v.state}${v.flagged ? ` is-${v.flagged}` : ""}${v.run.substituted ? " is-substituted" : ""}${hoverId === v.run.id ? " is-hover" : ""}`}
          style={{ left: v.css.x, top: v.css.y, width: Math.max(v.css.w, 1), height: Math.max(v.css.h, 1) }}
          tabIndex={v.run.id === rovingId ? 0 : -1}
          aria-label={v.label}
          aria-disabled={v.state === "refused" ? true : undefined}
          aria-describedby={v.describe ? `${uid}-d${i}` : undefined}
          onFocus={() => v.run.id !== focusedId && props.onFocusedChange(v.run.id, false)}
          onKeyDown={(e) => onRunKeyDown(e, v)}
          onKeyUp={(e) => e.key === " " && e.preventDefault()}
          onClick={(e) => {
            e.stopPropagation();
            if (Date.now() - keyEcho.current > KEY_ECHO_MS) activate(v, "all");
          }}
        >
          {v.flagged && <Icon name="alertTriangle" size={12} className="st-run__flag" />}
          {v.run.substituted && v.state !== "refused" && <span className="st-run__badge" aria-hidden>i</span>}
          {v.state === "edited" && <span className="st-run__dot" aria-hidden />}
        </button>
      ))}
      <div hidden>
        {shown.map((v, i) => (v.describe ? <span key={v.run.id} id={`${uid}-d${i}`}>{v.describe}</span> : null))}
      </div>
      {tip && tipText(tip) && (
        <div className="st-tooltip" role="tooltip" style={{ left: tip.css.x, top: tip.css.y + tip.css.h + 4 }}>
          {tipText(tip)}
        </div>
      )}
    </div>
  );
}

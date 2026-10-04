/**
 * Inline editor for one line (SPEC §D.5 EDITING, §D.6.1): a single-line input
 * over the run, width guides, a one-line message row and the format bar.
 *
 * Done normalises the draft; a no-op restores the original (if changed) and
 * closes; a local blocking problem keeps the editor open; anything else is
 * checked by `preview_text_edits` together with the page's other changes and
 * committed (one history step) only when that check passes. A failed check
 * keeps the draft and shows why (B11). STALE / PDF_NEEDS_REPAIR close the
 * editor with nothing committed; the page banner explains.
 */
import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import {
  blockingVerdict,
  displayedSize,
  editsSignature,
  estimateDeltaPt,
  isNoOpEdit,
  makeMapping,
  makeSourceTextObject,
  normaliseStyle,
  normaliseTyped,
  overlapsNext,
  pdfRectToViewport,
  pdfToViewport,
  problemMessage,
  toTextEditIn,
  type SetSourceTextInput,
  type SourceTextObject,
} from "@/lib/editor";
import { PROBLEM_COPY, UI, fillCopy, problemCopy, warningCopy, widthSentence } from "@/lib/editor/sourceTextCopy";
import { toAppError, type AppError, type SourceTextStyle, type TextEditIn, type TextFont, type TextPreview, type TextRun } from "@/lib/types";
import type { PageLayout } from "../PageSurface";
import { TextFormatBar, chipFont, steppedSize, toggledFace, type StyleChange } from "./TextFormatBar";

const CHIP_PAD_PX = 3;
const LINE_HEIGHT = 1.3;
/** The visible-limit guide appears once the new end is this close to it. */
const LIMIT_GUIDE_PT = 10;
const ASSERTIVE_MS = 1500;
/** Room the bar and message row need above / below the chip (CSS px). */
const EDGE_ROOM_PX = 44;

export interface DoneOptions {
  /** Return focus to the run after closing (Enter / Done); not after an outside click. */
  focusRun: boolean;
  /** Add "Fix or cancel this change first…" when the change can't complete. */
  blockedNotice: boolean;
}
export type TryDone = (opts: DoneOptions) => Promise<boolean>;

export interface SourceTextEditorProps {
  run: TextRun;
  fonts: Map<string, TextFont>;
  layout: PageLayout;
  pageIndex: number;
  sourceFingerprint: string;
  sourcePageIndex: number;
  fileName: string;
  /** The committed change of this run, if any. */
  existing: SourceTextObject | null;
  /** The page's other committed changes (checked together with this one). */
  others: SourceTextObject[];
  /** Caret index (code points) or select everything. */
  caret: number | "all";
  /** Text of the next run on the line, for the overlap message. */
  neighbourText: string | null;
  requestPreview: (edits: TextEditIn[]) => Promise<TextPreview>;
  seedPreview: (signature: string, preview: TextPreview) => void;
  onApply: (input: SetSourceTextInput) => void;
  onRevert: (id: string) => void;
  onClose: (outcome: { focusRun: boolean; announce: string | null }) => void;
  onFileError: (error: AppError) => void;
  registerTryDone: (fn: TryDone | null) => void;
}

/** UTF-16 offset of the caret after `index` code points. */
function utf16Index(text: string, index: number): number {
  return Array.from(text).slice(0, index).join("").length;
}

function styleKey(text: string, style: SourceTextStyle): string {
  return JSON.stringify([text, style.sizePt, style.face, style.fill, style.letterSpacingPt]);
}

function failureMessage(preview: TextPreview, runId: string, run: TextRun, fonts: Map<string, TextFont>, name: string): string {
  const verdict = preview.verdicts.find((v) => v.runId === runId);
  if (verdict && !verdict.ok) return problemMessage(verdict, run, fonts, name);
  if (preview.pageProblem) return problemCopy(preview.pageProblem.code, [], { name });
  return PROBLEM_COPY.EDIT_VERIFY_FAILED;
}

export function SourceTextEditor(props: SourceTextEditorProps) {
  const { run, fonts, layout, existing } = props;
  const metrics = run.metrics;
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const chipRef = useRef<HTMLDivElement>(null);
  const mirrorRef = useRef<HTMLSpanElement>(null);
  const msgId = useId();
  /** Rendered width of the draft in the input's font, and the stage room around the chip. */
  const [measured, setMeasured] = useState({ textPx: 0, above: Infinity, below: Infinity });
  const [draft, setDraft] = useState(existing?.text ?? run.text);
  const [draftStyle, setDraftStyle] = useState<SourceTextStyle>(existing?.style ?? {});
  const [notice, setNotice] = useState<string | null>(null);
  const [server, setServer] = useState<{ key: string; message: string } | null>(null);
  const [checking, setChecking] = useState(false);
  const [blockedNote, setBlockedNote] = useState(false);
  const [assertive, setAssertive] = useState(false);
  const pending = useRef<Promise<boolean> | null>(null);
  const generation = useRef(0);
  const closed = useRef(false);
  const mounted = useRef(true);
  const assertiveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    // Set here too: StrictMode mounts, unmounts and mounts again in development.
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (assertiveTimer.current) clearTimeout(assertiveTimer.current);
    };
  }, []);

  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.focus();
    if (props.caret === "all") el.select();
    else {
      const at = utf16Index(el.value, props.caret);
      el.setSelectionRange(at, at);
    }
  }, []); // placed once, when the editor opens

  // After every render: the substitute font can be wider than the PDF estimate, and the
  // bar and message row must stay inside the visible part of the stage.
  useLayoutEffect(() => {
    const chip = chipRef.current?.getBoundingClientRect();
    const stage = rootRef.current?.closest(".pdf-editor__stage")?.getBoundingClientRect();
    const next = {
      textPx: Math.ceil(mirrorRef.current?.offsetWidth ?? 0),
      above: chip && stage ? Math.round(chip.top - stage.top) : Infinity,
      below: chip && stage ? Math.round(stage.bottom - chip.bottom) : Infinity,
    };
    if (next.textPx !== measured.textPx || next.above !== measured.above || next.below !== measured.below) setMeasured(next);
  });

  const close = (outcome: { focusRun: boolean; announce: string | null }) => {
    closed.current = true;
    props.onClose(outcome);
  };

  const block = (withNotice: boolean) => {
    setBlockedNote(withNotice);
    setAssertive(true);
    if (assertiveTimer.current) clearTimeout(assertiveTimer.current);
    assertiveTimer.current = setTimeout(() => mounted.current && setAssertive(false), ASSERTIVE_MS);
    inputRef.current?.focus();
  };

  const check = async (text: string, style: SourceTextStyle, opts: DoneOptions, gen: number): Promise<boolean> => {
    const candidate: TextEditIn = { runId: run.id, originalText: run.text, text, style };
    try {
      const preview = await props.requestPreview([...props.others.map(toTextEditIn), candidate]);
      if (gen !== generation.current || !mounted.current) return closed.current || !mounted.current;
      const verdict = preview.verdicts.find((v) => v.runId === run.id);
      if (verdict?.ok && !preview.pageProblem) {
        const fields = {
          runId: run.id,
          sourceFingerprint: props.sourceFingerprint,
          sourcePageIndex: props.sourcePageIndex,
          originalText: run.text,
          text,
          style,
        };
        const committed = makeSourceTextObject(existing?.id ?? run.id, props.pageIndex, verdict.newRect ?? run.rect, fields);
        props.seedPreview(editsSignature([...props.others, committed]), preview);
        props.onApply({ ...fields, pageIndex: props.pageIndex, run, rect: verdict.newRect });
        close({ focusRun: opts.focusRun, announce: fillCopy(UI.announce.applied, { width: widthSentence(verdict.deltaPt, true) }) });
        return true;
      }
      setServer({ key: styleKey(text, style), message: failureMessage(preview, run.id, run, fonts, props.fileName) });
    } catch (e) {
      if (gen !== generation.current || !mounted.current) return closed.current || !mounted.current;
      const err = toAppError(e);
      if (err.code === "STALE" || err.code === "PDF_NEEDS_REPAIR") {
        props.onFileError(err);
        close({ focusRun: false, announce: null });
        return true;
      }
      setServer({ key: styleKey(text, style), message: [err.message, err.suggestion].filter(Boolean).join(" ") });
    }
    setChecking(false);
    block(opts.blockedNotice);
    return false;
  };

  const done: TryDone = async (opts) => {
    if (closed.current) return true;
    if (pending.current) return pending.current;
    const text = normaliseTyped(draft);
    const style = normaliseStyle(run, draftStyle);
    if (isNoOpEdit(run, text, style)) {
      if (existing) props.onRevert(existing.id);
      close({ focusRun: opts.focusRun, announce: existing ? UI.announce.restored : null });
      return true;
    }
    if (blockingVerdict(run, fonts, text, style)) {
      block(opts.blockedNotice);
      return false;
    }
    const gen = ++generation.current;
    setChecking(true);
    const work = check(text, style, opts, gen).finally(() => {
      if (pending.current === work) pending.current = null;
    });
    pending.current = work;
    return work;
  };
  const doneRef = useRef(done);
  doneRef.current = done;

  const { registerTryDone } = props;
  useEffect(() => {
    registerTryDone((opts) => doneRef.current(opts));
    return () => registerTryDone(null);
  }, [registerTryDone]);

  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element | null;
      if (!target || rootRef.current?.contains(target) || target.classList?.contains("pdf-editor__stage")) return;
      void doneRef.current({ focusRun: false, blockedNotice: true });
    };
    document.addEventListener("pointerdown", onDown, true);
    return () => document.removeEventListener("pointerdown", onDown, true);
  }, []);

  const cancel = () => {
    generation.current += 1;
    pending.current = null;
    close({ focusRun: true, announce: null });
  };

  if (!metrics || !run.style) return null;

  const restyle = (change: StyleChange) => {
    if (checking) return; // the check in flight is for the style it started with
    setServer(null);
    setBlockedNote(false);
    if (change.style) {
      setDraftStyle(change.style);
      setNotice(null);
    } else if (change.notice) setNotice(change.notice);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const mod = e.metaKey || e.ctrlKey;
    // An IME owns Enter and Esc while composing (WebKit reports the confirming Enter as keyCode 229).
    if ((e.key === "Enter" || e.key === "Escape") && (e.nativeEvent.isComposing || e.keyCode === 229)) return;
    if (e.key === "Enter") {
      e.preventDefault();
      void done({ focusRun: true, blockedNotice: false });
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    } else if (e.key === "Tab" && !e.shiftKey) {
      e.preventDefault();
      rootRef.current?.querySelector<HTMLElement>('.st-bar [data-roving][tabindex="0"]')?.focus();
    } else if (mod && !e.shiftKey && (e.key === "b" || e.key === "B" || e.key === "i" || e.key === "I")) {
      e.preventDefault();
      restyle(toggledFace(run, draftStyle, e.key.toLowerCase() === "b" ? "bold" : "italic"));
    } else if (mod && e.shiftKey && (e.code === "Period" || e.code === "Comma" || ">.<,".includes(e.key))) {
      e.preventDefault();
      const up = e.code === "Period" || e.key === ">" || e.key === ".";
      restyle(steppedSize(run, draftStyle, up ? 0.5 : -0.5));
    }
  };

  // Geometry: the run's box on screen (upright as displayed, §A.5), font sized by its effective size.
  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);
  const pxPerPt = layout.cssHeight / Math.max(displayedSize(layout.geometry).h, 1);
  const box = pdfRectToViewport(run.rect, mapping);
  const font = chipFont(run, fonts, draftStyle, pxPerPt);
  const fontPx = Number(font.css.fontSize);
  const delta = estimateDeltaPt(run, fonts, draft, draftStyle);
  const endPt = metrics.originalWidth + delta;
  const left = box.x - CHIP_PAD_PX;
  const height = Math.max(box.h, fontPx * LINE_HEIGHT) + 2 * CHIP_PAD_PX;
  const top = box.y + box.h / 2 - height / 2;
  const width = Math.max(Math.max(metrics.originalWidth, endPt) * pxPerPt, measured.textPx + 2) + 2 * CHIP_PAD_PX;
  const along = (d: number) => pdfToViewport({ x: run.origin.x + run.dir.x * d, y: run.origin.y + run.dir.y * d }, mapping).x - left;
  const nearTop = Math.min(top, measured.above) < EDGE_ROOM_PX;
  const nearBottom = Math.min(layout.cssHeight - top - height, measured.below) < EDGE_ROOM_PX;

  // Message row, one line, highest priority first (§D.6.1).
  const text = normaliseTyped(draft);
  const blocked = blockingVerdict(run, fonts, text, draftStyle);
  const serverMsg = server && server.key === styleKey(text, normaliseStyle(run, draftStyle)) ? server.message : null;
  let tone: "danger" | "warning" | "neutral" = "neutral";
  let message: string;
  if (notice) [tone, message] = ["warning", notice];
  else if (checking) message = UI.status.checking;
  else if (serverMsg) [tone, message] = ["danger", serverMsg];
  else if (blocked) [tone, message] = ["danger", problemMessage(blocked, run, fonts, props.fileName)];
  else if (overlapsNext(run, fonts, text, draftStyle)) [tone, message] = ["warning", warningCopy("NEXT_TEXT_OVERLAP", props.neighbourText)];
  else if (text === "") message = UI.removal;
  else message = widthSentence(delta, false) + (font.floored ? ` ${UI.shownLarger}` : "");
  if (blockedNote && tone === "danger") message = `${message} ${UI.blocked}`;

  const bar = (
    <TextFormatBar
      run={run}
      fonts={fonts}
      style={draftStyle}
      onStyle={(next) => restyle({ style: next })}
      onNotice={(sentence) => restyle({ notice: sentence })}
      canRestore={!!existing}
      onRestore={() => {
        generation.current += 1;
        if (existing) props.onRevert(existing.id);
        close({ focusRun: true, announce: UI.announce.restored });
      }}
      onCancel={cancel}
      onDone={() => void done({ focusRun: true, blockedNotice: false })}
      onBackToInput={() => inputRef.current?.focus()}
      placement={nearTop ? "below" : "above"}
    />
  );
  const row = (
    <div id={msgId} className={`st-msg st-msg--${tone}`} role="status" aria-live={assertive ? "assertive" : "polite"}>
      {message}
    </div>
  );

  // The bar floats above the chip and the message row sits below it; near the
  // page top the bar moves below the row, near the bottom the row moves above.
  return (
    <div ref={rootRef} className="st-editor" data-source-text="" style={{ left, top, width }}>
      <div className="st-dock st-dock--above">
        {!nearTop && bar}
        {nearBottom && row}
      </div>
      <div ref={chipRef} className="st-chip" style={{ height }}>
        <input
          ref={inputRef}
          type="text"
          className="st-chip__input"
          value={draft}
          readOnly={checking}
          spellCheck={false}
          aria-label={UI.editor.aria}
          aria-describedby={msgId}
          style={font.css}
          onChange={(e) => {
            setDraft(e.target.value);
            setNotice(null);
            setServer(null);
            setBlockedNote(false);
          }}
          onKeyDown={onKeyDown}
        />
        <span ref={mirrorRef} className="st-chip__mirror" style={font.css} aria-hidden="true">
          {draft}
        </span>
        <svg className="st-guides" width={Math.max(width, along(metrics.visibleExtent) + 2)} height={height} aria-hidden="true">
          <line className="st-guide st-guide--end" x1={along(endPt)} x2={along(endPt)} y1={0} y2={height} />
          {endPt < metrics.originalWidth - 0.01 && (
            <line className="st-guide st-guide--old" x1={along(metrics.originalWidth)} x2={along(metrics.originalWidth)} y1={0} y2={height} />
          )}
          {metrics.visibleExtent - endPt <= LIMIT_GUIDE_PT && (
            <line className="st-guide st-guide--limit" x1={along(metrics.visibleExtent)} x2={along(metrics.visibleExtent)} y1={0} y2={height} />
          )}
        </svg>
      </div>
      <div className="st-dock st-dock--below">
        {!nearBottom && row}
        {nearTop && bar}
      </div>
    </div>
  );
}

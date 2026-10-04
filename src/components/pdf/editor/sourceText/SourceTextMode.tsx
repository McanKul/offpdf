/**
 * Edit text for the shown page (SPEC §D.1, §D.4, §D.5, §D.7).
 *
 * - `useSourceTextPage` (used by the canvas for every tool): opens the file for
 *   Edit text when needed, reads the page's lines, keeps the real preview of
 *   the page's committed changes, and turns STALE / PDF_NEEDS_REPAIR answers
 *   into the file's banner state.
 * - `SourceTextBanners` (above the page) and `PreviewStatusChip` (on the page).
 * - `SourceTextMode` (on the page, tool = Edit text): the line layer, the
 *   inline editor and the reason popover, plus the live announcements.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import { Alert } from "@/components/ui/Alert";
import { Spinner } from "@/components/ui/Spinner";
import { fontsByKey, makeMapping, pdfRectToViewport, textStampGeometry, type EditObject, type SourceTextObject } from "@/lib/editor";
import type { TextStamp } from "@/lib/editor/sourceText";
import { UI, fileBanner, fillCopy, reasonCopy } from "@/lib/editor/sourceTextCopy";
import type { AppError, TextFont, TextRun } from "@/lib/types";
import type { TextSourceState, TextSources } from "@/features/edit-pdf/useTextSources";
import type { PageLayout } from "../PageSurface";
import type { EditSession } from "../useEditSession";
import { ReasonPopover } from "./ReasonPopover";
import { SourceTextEditor, type TryDone } from "./SourceTextEditor";
import { SourceTextLayer, nextOnLine } from "./SourceTextLayer";
import { usePageText, type PageTextState } from "./usePageText";
import { useTextPreview, type TextPreviewController } from "./useTextPreview";

const NO_CHANGES: SourceTextObject[] = [];
const FILE_FAULTS = new Set(["STALE", "PDF_NEEDS_REPAIR"]);

/** Lets the canvas finish (or keep) an open line edit before a page or tool change. */
export interface SourceTextGuard {
  isEditing(): boolean;
  tryClose(): Promise<boolean>;
}

/** Focus a line (and optionally open its editor) from the sidebar or inspector. */
export interface TextRequest {
  runId: string;
  open: boolean;
  tick: number;
}

export interface SourceTextPageArgs {
  /** Every object of the session (text changes on other pages decide staleness too). */
  objects: EditObject[];
  path: string;
  fileName: string;
  /** 1-based page in `path`. */
  sourcePage: number;
  /** 0-based page in the editor. */
  pageIndex: number;
  sources: TextSources;
  /** The Edit text tool is active. */
  active: boolean;
  /** The same source page is listed more than once. */
  duplicate: boolean;
}

export interface SourceTextPage {
  path: string;
  fileName: string;
  pageIndex: number;
  sourcePageIndex: number;
  active: boolean;
  duplicate: boolean;
  source: TextSourceState | null;
  /** Fingerprint the page's changes and lines belong to, when usable. */
  fingerprint: string | null;
  pageText: PageTextState;
  fonts: Map<string, TextFont>;
  /** This page's text changes. */
  objects: SourceTextObject[];
  /** Changes were made on a version of the file that is gone. */
  staleFingerprint: string | null;
  preview: TextPreviewController;
  /** A real preview of this page's text changes is on screen, so Show original has something to compare. */
  canShowOriginal: boolean;
  /** An inspect or preview error that is not a file state (shown as is). */
  problem: AppError | null;
  reportFault: (error: AppError) => void;
}

export function useSourceTextPage(args: SourceTextPageArgs): SourceTextPage {
  const { path, pageIndex, active, sources } = args;
  const { ensure, markStale, markError, reopen } = sources;
  const sourcePageIndex = args.sourcePage - 1;
  const objects = useMemo(
    () => args.objects.filter((o): o is SourceTextObject => o.kind === "sourceText" && o.pageIndex === pageIndex),
    [args.objects, pageIndex],
  );
  const source = sources.sources[path] ?? null;
  const info = source?.status === "ready" ? source.info : null;
  const wanted = active || objects.length > 0;
  useEffect(() => {
    if (wanted) void ensure(path);
  }, [wanted, path, ensure]);

  const mismatch = info ? objects.find((o) => o.sourceFingerprint !== info.fingerprint) : undefined;
  const staleFingerprint = source?.stale ? (info?.fingerprint ?? null) : (mismatch?.sourceFingerprint ?? null);
  const fingerprint = info && !source?.stale && !mismatch ? info.fingerprint : null;
  const pageText = usePageText({
    path,
    fingerprint,
    sourcePageIndex,
    enabled: wanted,
    pageCount: info?.pageCount ?? null,
  });
  const preview = useTextPreview({ path, fingerprint, sourcePageIndex, objects: fingerprint ? objects : NO_CHANGES });

  const reportFault = useCallback(
    (error: AppError) => {
      if (error.code === "STALE") markStale(path);
      else if (error.code === "PDF_NEEDS_REPAIR") markError(path, error);
    },
    [path, markStale, markError],
  );
  const fault = pageText.error ?? preview.error;
  useEffect(() => {
    if (fault) reportFault(fault);
  }, [fault, reportFault]);

  // A file that changed on disk with no text changes left on it is simply read again.
  const changesOnStale = staleFingerprint
    ? args.objects.some((o) => o.kind === "sourceText" && o.sourceFingerprint === staleFingerprint)
    : false;
  const stale = !!source?.stale;
  useEffect(() => {
    if (stale && !changesOnStale) void reopen(path);
  }, [stale, changesOnStale, path, reopen]);

  const fonts = useMemo(() => fontsByKey(pageText.page?.fonts ?? []), [pageText.page]);
  const problem = fault && !FILE_FAULTS.has(fault.code) ? fault : null;
  return {
    path,
    fileName: args.fileName,
    pageIndex,
    sourcePageIndex,
    active,
    duplicate: args.duplicate,
    source,
    fingerprint,
    pageText,
    fonts,
    objects,
    staleFingerprint,
    preview,
    canShowOriginal: objects.length > 0 && preview.bytes !== null,
    problem,
    reportFault,
  };
}

function errorText(error: AppError): string {
  return [error.message, error.suggestion].filter(Boolean).join(" ");
}

export function SourceTextBanners({
  page,
  onRemoveStale,
  modeDismissed,
  onDismissMode,
}: {
  page: SourceTextPage;
  onRemoveStale: (fingerprint: string) => void;
  modeDismissed: boolean;
  onDismissMode: () => void;
}) {
  const { source, pageText } = page;
  const text = pageText.page;
  const stale = page.staleFingerprint;
  const fileError = source?.status === "error" ? source.error : null;
  const loading = source?.status === "loading" || pageText.status === "loading";
  return (
    <div className="st-banners">
      {stale && (page.active || page.objects.length > 0) && (
        <Alert variant="warning">
          {fillCopy(UI.banner.stale, { name: page.fileName })}{" "}
          <button
            type="button"
            className="btn btn--secondary btn--sm st-banners__action"
            onClick={(e) => {
              // The banner goes away with the changes: keep focus in the editor (D.10), not on <body>.
              const editor = e.currentTarget.closest<HTMLElement>(".pdf-editor");
              onRemoveStale(stale);
              editor?.focus({ preventScroll: true });
            }}
          >
            {UI.banner.staleAction}
          </button>
        </Alert>
      )}
      {page.problem && (page.active || page.objects.length > 0) && (
        <Alert variant="warning" title={page.problem.title}>
          {errorText(page.problem)}
        </Alert>
      )}
      {page.active && fileError && <Alert variant="warning">{fileBanner(page.fileName, fileError)}</Alert>}
      {page.active && !fileError && loading && (
        <div className="st-banners__status" role="status">
          <Spinner /> {UI.status.reading}
        </div>
      )}
      {page.active && text && page.duplicate && <Alert variant="info">{UI.banner.duplicatePage}</Alert>}
      {page.active && text?.pageReason && (
        <Alert variant="warning">{fillCopy(UI.banner.pageRefused, { body: reasonCopy(text.pageReason).body })}</Alert>
      )}
      {page.active && text && !text.pageReason && text.runs.length === 0 && <Alert variant="info">{UI.banner.noText}</Alert>}
      {page.active && text && !text.pageReason && text.runs.length > 0 && !text.runs.some((r) => r.editable) && (
        <Alert variant="info">{UI.banner.noneEditable}</Alert>
      )}
      {page.active && !modeDismissed && (
        <Alert variant="info">
          <span className="st-banners__mode">
            {UI.banner.mode}
            <button type="button" className="btn btn--ghost btn--sm" onClick={onDismissMode}>
              {UI.popover.close}
            </button>
          </span>
        </Alert>
      )}
    </div>
  );
}

/** Top-right of the page while it has text changes (§D.7). */
export function PreviewStatusChip({ page, showOriginal }: { page: SourceTextPage; showOriginal: boolean }) {
  if (page.objects.length === 0 || !page.fingerprint) return null;
  const status = page.preview.status;
  const label = showOriginal
    ? UI.status.showingOriginal
    : status === "pending"
      ? UI.status.checking
      : status === "ready"
        ? UI.status.showingChanges
        : status === "unavailable"
          ? UI.status.previewUnavailable
          : null;
  if (!label) return null;
  return (
    <div className={`st-chip-status${status === "unavailable" && !showOriginal ? " is-muted" : ""}`} role="status">
      {status === "pending" && !showOriginal && <Spinner />}
      {label}
    </div>
  );
}

export interface SourceTextModeProps {
  page: SourceTextPage;
  layout: PageLayout;
  session: EditSession;
  stage: HTMLElement | null;
  /** Space-to-pan is active. */
  inert: boolean;
  guardRef: MutableRefObject<SourceTextGuard | null>;
  request: TextRequest | null;
  /** The request was acted on; the canvas drops it so a later mount never replays it. */
  onRequestHandled: () => void;
  /** Esc in the layer: focus goes back to the editor. */
  onLeave: () => void;
  /** "Add text here": the new box for the refused line (`textStampGeometry`). */
  onAddTextHere: (stamp: TextStamp) => void;
}

export function SourceTextMode(props: SourceTextModeProps) {
  const { page, layout, session, stage, inert, guardRef, request, onRequestHandled, onLeave, onAddTextHere } = props;
  const text = page.fingerprint ? page.pageText.page : null;
  const runs = useMemo(() => text?.runs ?? [], [text]);
  const [focus, setFocus] = useState<{ id: string | null; tick: number }>({ id: null, tick: 0 });
  const [editing, setEditing] = useState<{ runId: string; caret: number | "all" } | null>(null);
  const [popoverId, setPopoverId] = useState<string | null>(null);
  const [said, setSaid] = useState({ text: "", n: 0 });
  const tryDone = useRef<TryDone | null>(null);
  const editingRef = useRef(editing);
  editingRef.current = editing;
  const handled = useRef<TextRequest | null>(null); // by identity: the canvas restarts its tick after clearing

  const announce = (sentence: string) => setSaid((s) => ({ text: sentence, n: s.n + 1 }));
  const focusRun = useCallback((id: string, move: boolean) => setFocus((f) => ({ id, tick: move ? f.tick + 1 : f.tick })), []);
  const finishEditing = useCallback(
    () => (tryDone.current ? tryDone.current({ focusRun: false, blockedNotice: true }) : Promise.resolve(true)),
    [],
  );
  const registerTryDone = useCallback((fn: TryDone | null) => {
    tryDone.current = fn;
  }, []);

  useEffect(() => {
    guardRef.current = { isEditing: () => editingRef.current !== null, tryClose: finishEditing };
    return () => {
      guardRef.current = null;
    };
  }, [guardRef, finishEditing]);

  const edit = useCallback(
    async (run: TextRun, caret: number | "all") => {
      if (editingRef.current?.runId === run.id) return;
      if (editingRef.current && !(await finishEditing())) return;
      setPopoverId(null);
      setEditing({ runId: run.id, caret });
      focusRun(run.id, false);
    },
    [finishEditing, focusRun],
  );

  const explain = async (run: TextRun) => {
    if (editingRef.current && !(await finishEditing())) return;
    setPopoverId(run.id);
    focusRun(run.id, false);
  };

  useEffect(() => {
    if (!request || request === handled.current || !text) return;
    handled.current = request;
    onRequestHandled();
    const run = text.runs.find((r) => r.id === request.runId);
    if (!run) return;
    if (request.open && run.editable && !page.duplicate) void edit(run, "all");
    else focusRun(run.id, true);
  }, [request, text, page.duplicate, edit, focusRun, onRequestHandled]);

  if (!text || text.pageReason) return null;
  const byRun = new Map(page.objects.map((o) => [o.runId, o]));
  const editRun = editing ? (runs.find((r) => r.id === editing.runId) ?? null) : null;
  const popRun = popoverId ? (runs.find((r) => r.id === popoverId) ?? null) : null;
  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);

  return (
    <>
      <SourceTextLayer
        runs={runs}
        objects={page.objects}
        fonts={page.fonts}
        layout={layout}
        pageNumber={page.pageIndex + 1}
        preview={page.preview.result}
        duplicate={page.duplicate}
        focusedId={focus.id}
        focusTick={focus.tick}
        editingId={editing?.runId ?? null}
        popoverId={popoverId}
        stage={stage}
        inert={inert}
        onFocusedChange={focusRun}
        onEdit={(run, caret) => void edit(run, caret)}
        onExplain={(run) => void explain(run)}
        onRestore={(obj) => {
          session.revertSourceText(obj.id);
          announce(UI.announce.restored);
        }}
        onLeave={onLeave}
      />
      {editRun && page.fingerprint && editing && (
        <SourceTextEditor
          key={editRun.id}
          run={editRun}
          fonts={page.fonts}
          layout={layout}
          pageIndex={page.pageIndex}
          sourceFingerprint={page.fingerprint}
          sourcePageIndex={page.sourcePageIndex}
          fileName={page.fileName}
          existing={byRun.get(editRun.id) ?? null}
          others={page.objects.filter((o) => o.runId !== editRun.id)}
          caret={editing.caret}
          neighbourText={nextOnLine(runs, editRun)?.text ?? null}
          requestPreview={page.preview.request}
          seedPreview={page.preview.seed}
          onApply={session.setSourceText}
          onRevert={session.revertSourceText}
          onFileError={page.reportFault}
          registerTryDone={registerTryDone}
          onClose={(outcome) => {
            setEditing(null);
            if (outcome.announce) announce(outcome.announce);
            if (outcome.focusRun) focusRun(editRun.id, true);
          }}
        />
      )}
      {popRun && (
        <ReasonPopover
          run={popRun}
          anchor={pdfRectToViewport(popRun.rect, mapping)}
          pageWidth={layout.cssWidth}
          pageHeight={layout.cssHeight}
          onAddTextHere={() => {
            setPopoverId(null);
            onAddTextHere(textStampGeometry(popRun, layout.geometry));
          }}
          onClose={() => {
            setPopoverId(null);
            focusRun(popRun.id, true);
          }}
        />
      )}
      <div className="sr-only" role="status" aria-live="polite">
        {said.text}
        {said.n % 2 === 1 ? "​" : ""}
      </div>
    </>
  );
}

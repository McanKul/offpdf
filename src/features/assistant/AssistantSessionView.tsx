import { useEffect, useRef, type ReactNode } from "react";
import { Alert } from "@/components/ui/Alert";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Card } from "@/components/ui/Card";
import { Icon } from "@/components/ui/Icon";
import { Progress } from "@/components/ui/Progress";
import { Select } from "@/components/ui/Select";
import { Spinner } from "@/components/ui/Spinner";
import { formatCitation } from "@/lib/assistant/citations";
import type { Citation } from "@/lib/assistant/citations";
import type { AssistantModelStatus } from "@/lib/assistant/model";
import {
  hasSessionData,
  isSessionStale,
  type AssistantNoticeTurn,
  type AssistantSessionState,
  type AssistantTurn,
} from "@/lib/assistant/session";
import type { WorkspaceFile } from "@/lib/types";
import {
  canProcessDocuments,
  canRunAssistant,
  documentCountLabel,
  historyCountLabel,
  historyEmptyText,
  ingestionSummary,
  issueRows,
  modelStatusView,
  turnRequestLabel,
} from "./assistantView";

export interface AssistantSessionViewProps {
  readonly modelStatus: AssistantModelStatus;
  readonly session: AssistantSessionState;
  readonly files: readonly WorkspaceFile[];
  /** Document picker supplied by the page so this view stays Tauri-free. */
  readonly documents: ReactNode;
  readonly ocrEnabled: boolean;
  readonly ocrLanguage: string;
  readonly ocrLoading: boolean;
  readonly ocrAvailable: boolean;
  readonly ocrLanguages: readonly string[];
  readonly ocrError: string | null;
  readonly query: string;
  readonly onQueryChange: (query: string) => void;
  readonly onOcrEnabledChange: (enabled: boolean) => void;
  readonly onOcrLanguageChange: (language: string) => void;
  readonly onProcess: () => void;
  readonly onAsk: () => void;
  readonly onSummarize: () => void;
  readonly onCancel: () => void;
  readonly onOpenClear: () => void;
  readonly onOpenCitation: (citation: Citation) => void;
}

function ModelState({ status }: { status: AssistantModelStatus }) {
  const view = modelStatusView(status);
  return (
    <Card padded className="assistant-model">
      <div className="assistant-model__row">
        <div className="assistant-model__copy">
          <div className="assistant-model__icon">
            <Icon name="shield" size={20} />
          </div>
          <div>
            <div className="assistant-model__title">{view.title}</div>
            <div className="assistant-model__detail">{view.detail}</div>
            {status.backendId !== "none" && (
              <div className="assistant-model__backend">
                Backend: {status.backendId}
              </div>
            )}
          </div>
        </div>
        <Badge variant={view.badgeVariant}>{view.badgeLabel}</Badge>
      </div>
    </Card>
  );
}

function ProcessingState({
  session,
  onCancel,
}: {
  session: AssistantSessionState;
  onCancel: () => void;
}) {
  if (session.phase === "idle" || !session.progress) return null;
  const percent = session.progress.percent;
  return (
    <div
      className="assistant-progress"
      role="status"
      aria-live="polite"
      aria-label={session.progress.label}
    >
      <div className="row">
        <Spinner />
        <span>{session.progress.label}</span>
        {percent != null && (
          <span className="muted">{Math.round(percent)}%</span>
        )}
      </div>
      <Progress value={percent} />
      <div className="spread">
        <span className="muted">
          {session.phase === "answering"
            ? "The model runs on this device."
            : "Reading only the documents you selected."}
        </span>
        <Button
          variant="danger"
          size="sm"
          onClick={onCancel}
          leftIcon={<Icon name="stop" size={14} />}
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}

function Issues({ session }: { session: AssistantSessionState }) {
  const rows = issueRows(session.issues);
  if (rows.length === 0) return null;
  return (
    <div
      className="assistant-issues"
      role="list"
      aria-label="Document processing warnings"
    >
      {rows.map((row, index) => (
        <div className="assistant-issue" role="listitem" key={`${row.title}-${index}`}>
          <Icon name="alertTriangle" size={16} className="assistant-issue__icon" />
          <div>
            <div className="assistant-issue__title">
              {row.title}
              {row.where && <span className="muted"> - {row.where}</span>}
            </div>
            <div className="assistant-issue__message">{row.message}</div>
          </div>
        </div>
      ))}
    </div>
  );
}

function AnswerTurn({
  turn,
  onOpenCitation,
}: {
  turn: Extract<AssistantTurn, { kind: "answer" | "summary" }>;
  onOpenCitation: (citation: Citation) => void;
}) {
  return (
    <div className="assistant-turn__body">
      <p className="assistant-turn__text">{turn.answer.text}</p>
      {turn.answer.citations.length > 0 && (
        <div className="assistant-citations" aria-label="Sources">
          {turn.answer.citations.map((citation) => (
            <button
              type="button"
              className="assistant-citation"
              key={`${citation.chunkId}-${citation.page}`}
              onClick={() => onOpenCitation(citation)}
              title={`${formatCitation(citation)}: ${citation.quote}`}
              aria-label={`Open ${formatCitation(citation)} in the page viewer`}
            >
              <Icon name="fileText" size={13} />
              {formatCitation(citation)}
            </button>
          ))}
        </div>
      )}
      {turn.truncated && (
        <div className="assistant-turn__note">
          Some matching pages were omitted to stay within the local context
          limit.
        </div>
      )}
    </div>
  );
}

function NoticeTurn({
  turn,
}: {
  turn: Extract<AssistantTurn, { kind: "insufficient" | "error" }>;
}) {
  return (
    <Alert
      variant={turn.kind === "insufficient" ? "info" : "danger"}
      title={turn.kind === "insufficient" ? "Not enough evidence" : "No answer"}
    >
      {turn.message}
    </Alert>
  );
}

function HistoryTurn({
  turn,
  onOpenCitation,
}: {
  turn: AssistantTurn;
  onOpenCitation: (citation: Citation) => void;
}) {
  if (turn.kind === "answer" || turn.kind === "summary") {
    return (
      <article
        className="assistant-turn assistant-turn--answer"
        aria-label={turnRequestLabel(turn.kind, turn.request)}
      >
        <div className="assistant-turn__request">
          {turn.kind === "summary" ? "Summary" : turn.request ?? "Question"}
        </div>
        <AnswerTurn turn={turn} onOpenCitation={onOpenCitation} />
      </article>
    );
  }

  if (!isNoticeTurn(turn)) return null;

  const label =
    turn.requestKind === "summary"
      ? "Summary"
      : turn.request ?? (turn.kind === "error" ? "Request failed" : "Question");

  return (
    <article
      className="assistant-turn assistant-turn--notice"
      aria-label={label}
    >
      <div className="assistant-turn__request">{label}</div>
      <NoticeTurn turn={turn} />
    </article>
  );
}

function isNoticeTurn(turn: AssistantTurn): turn is AssistantNoticeTurn {
  return turn.kind === "insufficient" || turn.kind === "error";
}

export function AssistantSessionView({
  modelStatus,
  session,
  files,
  documents,
  ocrEnabled,
  ocrLanguage,
  ocrLoading,
  ocrAvailable,
  ocrLanguages,
  ocrError,
  query,
  onQueryChange,
  onOcrEnabledChange,
  onOcrLanguageChange,
  onProcess,
  onAsk,
  onSummarize,
  onCancel,
  onOpenClear,
  onOpenCitation,
}: AssistantSessionViewProps) {
  const model = modelStatusView(modelStatus);
  const canProcess = canProcessDocuments(session.phase, model.ready, files.length, {
    enabled: ocrEnabled,
    loading: ocrLoading,
    available: ocrAvailable,
    language: ocrLanguage,
  });
  const canAsk = canRunAssistant(
    session.phase,
    model.ready,
    session.chunks.length,
  );
  const summary = ingestionSummary(session.stats);
  const stale = isSessionStale(files, session.sources);
  const historyRef = useRef<HTMLDivElement>(null);

  // Move focus to the result region when a turn arrives, including for screen
  // readers. The log remains navigable with normal Tab/Shift+Tab.
  useEffect(() => {
    if (session.turns.length > 0) historyRef.current?.focus();
  }, [session.turns.length]);

  const submit = () => {
    if (canAsk && query.trim().length > 0) onAsk();
  };

  return (
    <div
      className="tool-layout assistant-session"
      aria-busy={session.phase !== "idle"}
    >
      <ModelState status={modelStatus} />

      <Card className="assistant-card">
        <div className="assistant-card__header">
          <div>
            <div className="card__title">Documents</div>
            <div className="card__desc">
              Only files you explicitly add are read. Folders are never indexed.
            </div>
          </div>
          <Badge variant="neutral">
            {documentCountLabel(files.length, session.sources.length)}
          </Badge>
        </div>
        <div className="assistant-card__body">
          {documents}

          {stale && (
            <Alert variant="warning" title="Workspace changed">
              The selected documents no longer match this session. Process again
              to refresh extracted text and citations, or clear the session.
            </Alert>
          )}

          <div className="assistant-ocr">
            <label className="assistant-check">
              <input
                type="checkbox"
                checked={ocrEnabled}
                disabled={!model.ready || session.phase !== "idle"}
                onChange={(event) => onOcrEnabledChange(event.target.checked)}
              />
              <span>
                Use OCR for pages without a text layer
                <span className="field__hint">
                  OCR is opt-in. It runs only on pages with no usable native
                  text and only after you process the documents.
                </span>
              </span>
            </label>

            {ocrEnabled && (
              <div className="assistant-ocr__options">
                {ocrLoading ? (
                  <div className="row gap-sm muted">
                    <Spinner /> Checking OCR languages...
                  </div>
                ) : ocrError ? (
                  <Alert variant="warning" title="OCR could not be checked">
                    {ocrError} You can still process native text without OCR.
                  </Alert>
                ) : !ocrAvailable ? (
                  <Alert variant="warning" title="OCR is not available">
                    Install Tesseract and a language pack to use OCR. Native text
                    still works without it.
                  </Alert>
                ) : ocrLanguages.length === 0 ? (
                  <Alert variant="warning" title="No OCR language pack found">
                    Native text still works. Install a Tesseract language pack
                    before using OCR.
                  </Alert>
                ) : (
                  <Select
                    label="OCR language"
                    id="assistant-ocr-language"
                    value={ocrLanguage}
                    onChange={onOcrLanguageChange}
                    options={ocrLanguages.map((code) => ({
                      value: code,
                      label: code,
                    }))}
                    hint="Choose an installed Tesseract language pack."
                  />
                )}
              </div>
            )}
          </div>

          <div className="assistant-card__actions">
            <Button
              variant="primary"
              onClick={onProcess}
              disabled={!canProcess}
              loading={session.phase === "processing"}
              leftIcon={<Icon name="scanText" size={15} />}
            >
              {session.phase === "processing"
                ? "Processing..."
                : session.chunks.length > 0
                  ? "Process again"
                  : "Process documents"}
            </Button>
            {!model.ready && (
              <span className="muted">
                Processing stays off until a local model is installed.
              </span>
            )}
            {model.ready && session.turns.length > 0 && (
              <span className="muted">
                Processing again replaces extracted text and history.
              </span>
            )}
          </div>

          {session.phase === "processing" && (
            <ProcessingState session={session} onCancel={onCancel} />
          )}

          {summary && (
            <div className="assistant-stats">
              {summary}
              {session.sources.length > 0 && (
                <span className="muted">
                  {" "}
                  - {session.chunks.length} searchable chunk
                  {session.chunks.length === 1 ? "" : "s"} kept in memory
                </span>
              )}
            </div>
          )}
          <Issues session={session} />
        </div>
      </Card>

      <Card className="assistant-card">
        <div className="assistant-card__header">
          <div>
            <div className="card__title">Ask or summarize</div>
            <div className="card__desc">
              Answers use only the processed pages. If the evidence is missing,
              the assistant says so.
            </div>
          </div>
          {session.pendingKind && (
            <Badge variant="info">
              {session.pendingKind === "summary" ? "Summarizing" : "Thinking"}
            </Badge>
          )}
        </div>
        <div className="assistant-card__body">
          <form
            className="assistant-question-form"
            onSubmit={(event) => {
              event.preventDefault();
              submit();
            }}
          >
            <label className="field__label" htmlFor="assistant-question">
              Question
            </label>
            <textarea
              id="assistant-question"
              className="input assistant-question"
              value={query}
              onChange={(event) => onQueryChange(event.target.value)}
              placeholder="What does this document say about...?"
              rows={3}
              disabled={!canAsk}
              aria-keyshortcuts="Control+Enter Meta+Enter"
              onKeyDown={(event) => {
                if (
                  event.key === "Enter" &&
                  (event.ctrlKey || event.metaKey)
                ) {
                  event.preventDefault();
                  submit();
                }
              }}
            />
            <div className="assistant-actions">
              <span className="field__hint">
                {canAsk
                  ? "Ctrl+Enter asks without leaving the keyboard."
                  : model.ready
                    ? "Process at least one document first."
                    : "The assistant is optional and needs a local model."}
              </span>
              <div className="row gap-sm">
                <Button
                  type="submit"
                  variant="primary"
                  disabled={!canAsk || query.trim().length === 0}
                  loading={session.pendingKind === "answer"}
                  leftIcon={<Icon name="bubble" size={15} />}
                >
                  Ask
                </Button>
                <Button
                  type="button"
                  variant="secondary"
                  onClick={onSummarize}
                  disabled={!canAsk}
                  loading={session.pendingKind === "summary"}
                  leftIcon={<Icon name="sparkles" size={15} />}
                >
                  Summarize
                </Button>
              </div>
            </div>
          </form>

          {session.phase === "answering" && (
            <ProcessingState session={session} onCancel={onCancel} />
          )}

          {session.error && (
            <Alert variant="danger" title="Request failed">
              {session.error}
            </Alert>
          )}
          {session.notice && (
            <Alert variant="info" title="Session notice">
              {session.notice}
            </Alert>
          )}
        </div>
      </Card>

      <Card className="assistant-card assistant-history-card">
        <div className="assistant-card__header">
          <div>
            <div className="card__title">Session history</div>
            <div className="card__desc">
              In memory only. Leaving this page or clearing the session removes
              it. Nothing is synced or uploaded.
            </div>
          </div>
          <div className="row gap-sm">
            <Badge variant="neutral">{historyCountLabel(session)}</Badge>
            <Button
              variant="ghost"
              size="sm"
              onClick={onOpenClear}
              disabled={!hasSessionData(session)}
              leftIcon={<Icon name="trash" size={14} />}
            >
              Clear session
            </Button>
          </div>
        </div>
        <div className="assistant-card__body">
          <div
            className="assistant-history"
            role="log"
            aria-live="polite"
            aria-relevant="additions"
            aria-label="Assistant answers and summaries"
            tabIndex={-1}
            ref={historyRef}
          >
            {session.turns.length === 0 ? (
              <div className="empty">
                <div className="empty__icon">
                  <Icon name="bubble" size={28} />
                </div>
                <div className="section-label">No answers yet</div>
                <div className="muted">
                  {historyEmptyText(model.ready, session.chunks.length)}
                </div>
              </div>
            ) : (
              session.turns.map((turn) => (
                <HistoryTurn
                  key={turn.id}
                  turn={turn}
                  onOpenCitation={onOpenCitation}
                />
              ))
            )}
          </div>
        </div>
      </Card>
    </div>
  );
}

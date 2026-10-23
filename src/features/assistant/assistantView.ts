/**
 * Pure view helpers for the private-assistant page. Keeping labels and states
 * here makes empty/loading/error behavior testable without a live model, Tauri
 * runtime or browser event loop.
 */

import { formatPageRanges } from "@/lib/assistant/citations";
import type {
  IngestionIssue,
  IngestionIssueCode,
  IngestionStats,
} from "@/lib/assistant/ingestion";
import type {
  AssistantModelStatus,
  AssistantModelState,
} from "@/lib/assistant/model";
import type {
  AssistantPhase,
  AssistantSessionState,
} from "@/lib/assistant/session";
import { formatCount } from "@/lib/formatBytes";

export type AssistantNoticeVariant =
  | "neutral"
  | "success"
  | "warning"
  | "danger"
  | "info";

export interface ModelStatusView {
  readonly ready: boolean;
  readonly alertVariant: "info" | "success" | "warning" | "danger";
  readonly badgeVariant: AssistantNoticeVariant;
  readonly badgeLabel: string;
  readonly title: string;
  readonly detail: string;
}

const MODEL_BADGES: Record<
  AssistantModelState,
  {
    badge: string;
    badgeVariant: AssistantNoticeVariant;
    alertVariant: ModelStatusView["alertVariant"];
  }
> = {
  ready: {
    badge: "Ready",
    badgeVariant: "success",
    alertVariant: "success",
  },
  "not-installed": {
    badge: "Optional",
    badgeVariant: "warning",
    alertVariant: "warning",
  },
  unsupported: {
    badge: "Unsupported",
    badgeVariant: "danger",
    alertVariant: "danger",
  },
  error: {
    badge: "Unavailable",
    badgeVariant: "danger",
    alertVariant: "danger",
  },
};

export function modelStatusView(status: AssistantModelStatus): ModelStatusView {
  const meta = MODEL_BADGES[status.state];
  return {
    ready: status.state === "ready",
    alertVariant: meta.alertVariant,
    badgeVariant: meta.badgeVariant,
    badgeLabel: meta.badge,
    title: status.title,
    detail: status.detail,
  };
}

const ISSUE_TITLES: Record<IngestionIssueCode, string> = {
  NO_SOURCES: "No documents selected",
  INVALID_SOURCE_ID: "Invalid document id",
  DUPLICATE_SOURCE_ID: "Duplicate document id",
  TOO_MANY_SOURCES: "Document limit",
  NOT_PDF: "Not a readable PDF",
  UNREADABLE_PDF: "Unreadable PDF",
  EMPTY_PDF: "No pages",
  SOURCE_TOO_LARGE: "Document too large",
  PAGE_LIMIT: "Page limit",
  EXTRACTION_FAILED: "Text extraction failed",
  TEXT_LIMIT: "Text limit",
  OCR_REQUIRED: "Scanned pages need OCR",
  OCR_LANGUAGE_REQUIRED: "OCR language needed",
  OCR_LIMIT: "OCR limit",
  OCR_EMPTY: "No OCR text",
  OCR_UNAVAILABLE: "OCR unavailable",
  OCR_FAILED: "OCR failed",
  INDEX_LIMIT: "Session text limit",
};

export interface IssueRow {
  readonly title: string;
  readonly where: string | null;
  readonly message: string;
}

export function issueWhere(issue: IngestionIssue): string | null {
  const parts: string[] = [];
  if (issue.sourceName) parts.push(issue.sourceName);
  if (issue.pages && issue.pages.length > 0) {
    parts.push(formatPageRanges(issue.pages));
  }
  return parts.length > 0 ? parts.join(", ") : null;
}

export function issueRows(issues: readonly IngestionIssue[]): IssueRow[] {
  return issues.map((issue) => ({
    title: ISSUE_TITLES[issue.code],
    where: issueWhere(issue),
    message: issue.message,
  }));
}

export function ingestionSummary(stats: IngestionStats | null): string | null {
  if (!stats) return null;
  const documents = `${formatCount(stats.sourceCount)} document${stats.sourceCount === 1 ? "" : "s"}`;
  const pages = `${formatCount(stats.pageCount)} page${stats.pageCount === 1 ? "" : "s"}`;
  const chunks = `${formatCount(stats.chunkCount)} search chunk${stats.chunkCount === 1 ? "" : "s"}`;
  const ocr = stats.ocrPages > 0 ? `, ${formatCount(stats.ocrPages)} OCR` : "";
  return `${documents}, ${pages}, ${chunks}${ocr}${stats.truncated ? " (limits applied)" : ""}`;
}

export function historyEmptyText(
  modelReady: boolean,
  chunkCount: number,
): string {
  if (!modelReady) {
    return "The assistant is off while no local model is installed. No text is read, uploaded, or kept.";
  }
  if (chunkCount === 0) {
    return "Process the selected documents, then ask a question or request a summary.";
  }
  return "Questions and summaries appear here. History stays in memory only.";
}

export function canRunAssistant(
  phase: AssistantPhase,
  modelReady: boolean,
  chunkCount: number,
): boolean {
  return phase === "idle" && modelReady && chunkCount > 0;
}

export function canProcessDocuments(
  phase: AssistantPhase,
  modelReady: boolean,
  fileCount: number,
  ocr: {
    readonly enabled: boolean;
    readonly loading: boolean;
    readonly available: boolean;
    readonly language: string;
  },
): boolean {
  if (phase !== "idle" || !modelReady || fileCount === 0) return false;
  if (!ocr.enabled) return true;
  return !ocr.loading && ocr.available && ocr.language.trim().length > 0;
}

export const CLEAR_SESSION_FACTS = [
  "Extracted page text and search chunks",
  "OCR text and citation quotes kept for this session",
  "Every question, summary, answer and error shown here",
  "Processing progress and document warnings",
] as const;

export const CLEAR_SESSION_KEEP = [
  "Your PDF files are not changed or deleted.",
  "The local model stays installed and loaded.",
  "Nothing was uploaded or saved by the assistant.",
] as const;

/** Exact, current-session counts shown before clearing. */
export function clearSessionSummary(state: AssistantSessionState): string {
  const count = (value: number, one: string, many: string) =>
    `${formatCount(value)} ${value === 1 ? one : many}`;
  const ocr = state.stats?.ocrPages ?? 0;
  return [
    `This removes ${count(state.sources.length, "processed document", "processed documents")}`,
    count(state.chunks.length, "search chunk", "search chunks"),
    ...(ocr > 0 ? [count(ocr, "OCR page", "OCR pages")] : []),
    `and ${count(state.turns.length, "history turn", "history turns")} from memory:`,
  ].join(", ").replace(", and", " and");
}

/** Exact confirmation copy used by the clear-session dialog. */
export function clearSessionFacts(): readonly string[] {
  return CLEAR_SESSION_FACTS;
}

export function clearSessionKeeps(): readonly string[] {
  return CLEAR_SESSION_KEEP;
}

export function turnRequestLabel(
  kind: "answer" | "summary" | "insufficient" | "error",
  request?: string,
): string {
  if (kind === "summary") return "Summary";
  if (request) return request;
  return kind === "error" ? "Request failed" : "Question";
}

export function historyCountLabel(state: AssistantSessionState): string {
  const count = state.turns.length;
  return `${formatCount(count)} turn${count === 1 ? "" : "s"}`;
}

export function documentCountLabel(
  fileCount: number,
  sourceCount: number,
): string {
  if (sourceCount > 0) {
    return `${formatCount(sourceCount)} processed`;
  }
  return `${formatCount(fileCount)} selected`;
}

/**
 * In-memory state and bounded execution for one private-assistant session.
 *
 * This module is deliberately free of React and Tauri imports. The session hook
 * supplies a model and an ingestion runtime; the reducer here owns exactly what
 * the UI may retain. Clearing a session drops extracted chunks, sources,
 * progress, errors and every question/answer turn. Nothing is persisted.
 */

import {
  GROUNDING_MESSAGES,
  groundAnswerJson,
  groundSummaryJson,
  type GroundedAnswer,
  type GroundingFailureReason,
  type GroundingResult,
} from "./citations";
import type {
  IngestionIssue,
  IngestionProgress,
  IngestionResult,
  IngestionStage,
  IngestedSource,
  IngestionStats,
} from "./ingestion";
import type { AssistantModel } from "./model";
import {
  retrieveChunks,
  selectSummaryChunks,
  type PageChunk,
} from "./retrieval";

export type AssistantPhase = "idle" | "processing" | "answering";
export type AssistantRequestKind = "answer" | "summary";

export interface AssistantProgress {
  readonly stage: IngestionStage | "thinking";
  /** 0-100, or null for an indeterminate operation. */
  readonly percent: number | null;
  readonly label: string;
}

export interface AssistantAnswerTurn {
  readonly id: string;
  readonly kind: "answer" | "summary";
  /** The user's question; omitted for a summary. */
  readonly request?: string;
  readonly answer: GroundedAnswer;
  readonly truncated: boolean;
}

export interface AssistantNoticeTurn {
  readonly id: string;
  readonly kind: "insufficient" | "error";
  readonly request?: string;
  /** The request family, so a failed summary is not labeled a question. */
  readonly requestKind?: AssistantRequestKind;
  readonly message: string;
  readonly reason?: GroundingFailureReason;
}

export type AssistantTurn = AssistantAnswerTurn | AssistantNoticeTurn;

export interface AssistantSessionState {
  readonly phase: AssistantPhase;
  readonly pendingKind: AssistantRequestKind | null;
  readonly sources: readonly IngestedSource[];
  readonly chunks: readonly PageChunk[];
  readonly issues: readonly IngestionIssue[];
  readonly stats: IngestionStats | null;
  readonly turns: readonly AssistantTurn[];
  readonly progress: AssistantProgress | null;
  readonly error: string | null;
  readonly notice: string | null;
}

export const INITIAL_ASSISTANT_SESSION: AssistantSessionState = {
  phase: "idle",
  pendingKind: null,
  sources: [],
  chunks: [],
  issues: [],
  stats: null,
  turns: [],
  progress: null,
  error: null,
  notice: null,
};

export type AssistantSessionAction =
  | { type: "start-processing" }
  | { type: "progress"; progress: AssistantProgress }
  | { type: "processing-done"; result: IngestionResult }
  | {
      type: "start-answering";
      kind: AssistantRequestKind;
      request?: string;
    }
  | { type: "turn"; turn: AssistantTurn }
  | { type: "fail"; message: string }
  | { type: "cancel" }
  | { type: "clear" };

export function assistantSessionReducer(
  state: AssistantSessionState,
  action: AssistantSessionAction,
): AssistantSessionState {
  switch (action.type) {
    case "start-processing":
      return {
        ...INITIAL_ASSISTANT_SESSION,
        phase: "processing",
        progress: {
          stage: "inspecting",
          percent: 0,
          label: "Preparing documents",
        },
      };

    case "progress":
      return { ...state, progress: action.progress };

    case "processing-done": {
      const { result } = action;
      return {
        ...state,
        phase: "idle",
        pendingKind: null,
        sources: result.sources,
        chunks: result.chunks,
        issues: result.issues,
        stats: result.stats,
        progress: null,
        error: null,
        notice:
          result.chunks.length === 0
            ? "No usable text was found. If these pages are scanned, enable OCR and process the documents again."
            : null,
      };
    }

    case "start-answering":
      return {
        ...state,
        phase: "answering",
        pendingKind: action.kind,
        progress: {
          stage: "thinking",
          percent: null,
          label:
            action.kind === "summary"
              ? "Summarizing the selected pages"
              : "Reading the selected pages",
        },
        error: null,
        notice: null,
      };

    case "turn":
      return {
        ...state,
        phase: "idle",
        pendingKind: null,
        turns: [...state.turns, action.turn],
        progress: null,
        error: null,
        notice: null,
      };

    case "fail":
      return {
        ...state,
        phase: "idle",
        pendingKind: null,
        progress: null,
        error: action.message,
      };

    case "cancel":
      return {
        ...state,
        phase: "idle",
        pendingKind: null,
        progress: null,
        error: null,
        notice: "Cancelled. No partial result was kept.",
      };

    case "clear":
      return INITIAL_ASSISTANT_SESSION;

    default:
      return state;
  }
}

const STAGE_LABELS: Record<IngestionStage, string> = {
  inspecting: "Inspecting",
  extracting: "Reading text from",
  ocr: "Running OCR on",
  chunking: "Preparing",
  done: "Finishing",
};

/** Map bounded ingestion progress to a short, non-path UI label. */
export function assistantIngestionProgress(
  progress: IngestionProgress,
  sourceNames: ReadonlyMap<string, string>,
): AssistantProgress {
  const name = sourceNames.get(progress.sourceId) ?? "document";
  const position = `${progress.sourceIndex + 1} of ${progress.sourceCount}`;
  const action = STAGE_LABELS[progress.stage];
  return {
    stage: progress.stage,
    percent: progress.percent,
    label:
      progress.stage === "done"
        ? "Finishing document processing"
        : `${action} ${name} (${position})`,
  };
}

export function throwIfAborted(signal: AbortSignal): void {
  if (!signal.aborted) return;
  const error = new Error("Assistant request cancelled.");
  error.name = "AbortError";
  throw error;
}

export function isAbortError(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    ((error as { name?: unknown }).name === "AbortError" ||
      (error as { code?: unknown }).code === "ABORT_ERR")
  );
}

export function assistantErrorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.trim().length > 0) {
      return message;
    }
    const title = (error as { title?: unknown }).title;
    if (typeof title === "string" && title.trim().length > 0) {
      return title;
    }
  }
  return "The local assistant could not complete that request.";
}

export function groundingTurn(
  id: string,
  request: { kind: AssistantRequestKind; text?: string },
  result: GroundingResult,
): AssistantTurn {
  if (result.ok) {
    return {
      id,
      kind: request.kind,
      request: request.text,
      answer: result.answer,
      truncated: result.retrieval.truncated,
    };
  }

  return {
    id,
    kind: result.reason === "no-evidence" ? "insufficient" : "error",
    request: request.text,
    requestKind: request.kind,
    message: result.message,
    reason: result.reason,
  };
}

export function noticeTurn(
  id: string,
  kind: "insufficient" | "error",
  message: string,
  options: {
    request?: string;
    requestKind?: AssistantRequestKind;
    reason?: GroundingFailureReason;
  } = {},
): AssistantNoticeTurn {
  return {
    id,
    kind,
    request: options.request,
    requestKind: options.requestKind,
    message,
    reason: options.reason,
  };
}

function insufficientTurn(
  id: string,
  request: { kind: AssistantRequestKind; text?: string },
): AssistantTurn {
  return noticeTurn(id, "insufficient", GROUNDING_MESSAGES.insufficientEvidence, {
    request: request.text,
    requestKind: request.kind,
    reason: "no-evidence",
  });
}

function noQueryTurn(id: string): AssistantTurn {
  return noticeTurn(id, "error", GROUNDING_MESSAGES.noQuery, {
    requestKind: "answer",
    reason: "no-query",
  });
}

/**
 * Retrieve, ask the model, then validate prose against exactly the retrieved
 * context. The model is never called when retrieval has no usable evidence.
 */
export async function runGroundedAnswer(
  id: string,
  query: string,
  chunks: readonly PageChunk[],
  model: AssistantModel,
  signal: AbortSignal,
): Promise<AssistantTurn> {
  throwIfAborted(signal);
  const retrieval = retrieveChunks(query, chunks);
  const request = { kind: "answer" as const, text: retrieval.query };

  if (retrieval.terms.length === 0) return noQueryTurn(id);
  if (retrieval.chunks.length === 0) return insufficientTurn(id, request);

  const json = await model.generate(
    {
      kind: "answer",
      query: retrieval.query,
      chunks: retrieval.chunks,
      truncated: retrieval.truncated,
    },
    signal,
  );
  throwIfAborted(signal);

  return groundingTurn(
    id,
    request,
    groundAnswerJson(retrieval.query, chunks, json),
  );
}

/**
 * Sample bounded summary context and ground the model's summary against it.
 * Summaries need at least one chunk; otherwise no model call is made.
 */
export async function runGroundedSummary(
  id: string,
  chunks: readonly PageChunk[],
  model: AssistantModel,
  signal: AbortSignal,
): Promise<AssistantTurn> {
  throwIfAborted(signal);
  const context = selectSummaryChunks(chunks);
  const request = { kind: "summary" as const };

  if (context.chunks.length === 0) return insufficientTurn(id, request);

  const json = await model.generate(
    {
      kind: "summary",
      query: "",
      chunks: context.chunks,
      truncated: context.truncated,
    },
    signal,
  );
  throwIfAborted(signal);

  return groundingTurn(id, request, groundSummaryJson(chunks, json));
}

/** True when workspace selected documents differ from the ingested sources. */
export function isSessionStale(
  files: readonly { readonly uid: string }[],
  sources: readonly IngestedSource[],
): boolean {
  if (sources.length === 0) return false;
  if (files.length !== sources.length) return true;
  const ingested = new Set(sources.map((source) => source.id));
  return files.some((file) => !ingested.has(file.uid));
}

export function hasSessionData(state: AssistantSessionState): boolean {
  return (
    state.sources.length > 0 ||
    state.chunks.length > 0 ||
    state.issues.length > 0 ||
    state.turns.length > 0
  );
}

/** Resolve a citation to the local source metadata used by the page viewer. */
export function citationSource(
  sources: readonly IngestedSource[],
  sourceId: string,
): IngestedSource | undefined {
  return sources.find((source) => source.id === sourceId);
}

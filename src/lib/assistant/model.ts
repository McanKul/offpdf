/**
 * Product-facing local model contract for the private assistant.
 *
 * The session UI does not import a concrete runtime. A runtime adapter supplies
 * an `AssistantModel`; until model setup and a real local runtime are available,
 * `getAssistantModel()` returns an explicitly unavailable model and the UI stays
 * clearly optional. No network fallback exists.
 *
 * The adapter may format its own prompt, but it must use only the chunks in the
 * request. The returned string is a versioned JSON answer envelope validated by
 * `citations.ts`; prose that cannot be grounded is discarded.
 */

import type { RetrievedChunk } from "./retrieval";

export type AssistantModelState =
  | "ready"
  | "not-installed"
  | "unsupported"
  | "error";

export interface AssistantModelStatus {
  readonly state: AssistantModelState;
  /** Stable non-path identifier, e.g. `fake-v1` or `llama-cpp`. */
  readonly backendId: string;
  readonly title: string;
  readonly detail: string;
}

export interface AssistantModelRequest {
  readonly kind: "answer" | "summary";
  readonly query: string;
  /**
   * Bounded, page-aligned context. These are the only chunks the model may
   * cite. `truncated` means retrieval omitted matches to stay in budget.
   */
  readonly chunks: readonly RetrievedChunk[];
  readonly truncated: boolean;
}

export interface AssistantModel {
  readonly id: string;
  readonly status: AssistantModelStatus;
  /**
   * Return the model's raw answer envelope. Implementations must cooperate with
   * `signal`; the session also discards a result that arrives after cancellation.
   */
  generate(
    request: AssistantModelRequest,
    signal: AbortSignal,
  ): Promise<string>;
}

export class AssistantModelUnavailableError extends Error {
  constructor() {
    super("No local model is installed.");
    this.name = "AssistantModelUnavailableError";
  }
}

export const NO_LOCAL_MODEL: AssistantModel = {
  id: "none",
  status: {
    state: "not-installed",
    backendId: "none",
    title: "No local model installed",
    detail:
      "The private assistant is optional. It stays off until a local model is available, and no document text has been read. OffPDF's PDF tools keep working normally.",
  },
  generate: async () => {
    throw new AssistantModelUnavailableError();
  },
};

/**
 * The current product default. Model setup and a real runtime can replace this
 * object when they land; nothing else in the session UI needs to change.
 */
export function getAssistantModel(): AssistantModel {
  return NO_LOCAL_MODEL;
}

export function isAssistantModelReady(model: AssistantModel): boolean {
  return model.status.state === "ready";
}

const PROMPT_HEADER = [
  "You are OffPDF's private local assistant.",
  "Use only the CONTEXT below. Do not use outside knowledge and do not invent document content.",
  "Every factual sentence must be supported by at least one citation.",
  "A citation names one context chunk id and quotes an exact span from that chunk.",
  'Return exactly one JSON object with this shape: {"version":1,"kind":"answer"|"summary"|"insufficient","text":"...","citations":[{"chunkId":"...","quote":"..."}]}',
  'If the context does not contain enough evidence, return kind "insufficient", explain that briefly in text, and use an empty citations array.',
  "Return JSON only: no markdown fences, no commentary before or after the object.",
].join("\n");

/**
 * Build the bounded prompt for a runtime adapter. Kept here so every backend
 * receives the same citation contract. It contains source names and page text
 * only; chunks never contain filesystem paths.
 */
export function buildAssistantPrompt(request: AssistantModelRequest): string {
  const task =
    request.kind === "summary"
      ? "Summarize the sampled pages. Highlight only what the context supports."
      : `Answer this question: ${request.query}`;

  const context = request.chunks
    .map(
      (chunk) =>
        `[${chunk.id}] ${chunk.sourceName}, page ${chunk.page}\n${chunk.text}`,
    )
    .join("\n\n---\n\n");

  return [
    PROMPT_HEADER,
    `TASK\n${task}`,
    `CONTEXT (${request.chunks.length} chunk${request.chunks.length === 1 ? "" : "s"}${request.truncated ? ", bounded" : ""})`,
    context,
  ].join("\n\n");
}

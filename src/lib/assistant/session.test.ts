import { describe, expect, it, vi } from "vitest";
import type { IngestionProgress, IngestionResult } from "./ingestion";
import type { AssistantModel, AssistantModelRequest } from "./model";
import {
  assistantIngestionProgress,
  assistantSessionReducer,
  citationSource,
  hasSessionData,
  INITIAL_ASSISTANT_SESSION,
  isSessionStale,
  runGroundedAnswer,
  runGroundedSummary,
} from "./session";
import type { PageChunk } from "./retrieval";

const chunks: PageChunk[] = [
  {
    id: "c1",
    sourceId: "f1",
    sourceName: "report.pdf",
    page: 1,
    text: "Revenue was 4.2 million euros in 2025.",
  },
  {
    id: "c2",
    sourceId: "f1",
    sourceName: "report.pdf",
    page: 2,
    text: "The board approved a new factory in Lisbon.",
  },
];

const stats = {
  sourceCount: 1,
  pageCount: 2,
  nativePages: 2,
  ocrPages: 0,
  emptyPages: 0,
  chunkCount: 2,
  chars: 90,
  truncated: false,
};

const result: IngestionResult = {
  chunks,
  sources: [
    {
      id: "f1",
      path: "C:\\private\\report.pdf",
      name: "report.pdf",
      pageCount: 2,
      consideredPages: 2,
      nativePages: 2,
      ocrPages: 0,
      emptyPages: 0,
    },
  ],
  issues: [],
  stats,
};

function readyModel(
  generate: (
    request: AssistantModelRequest,
    signal: AbortSignal,
  ) => Promise<string>,
): AssistantModel {
  return {
    id: "fake",
    status: {
      state: "ready",
      backendId: "fake-v1",
      title: "Local model ready",
      detail: "Test model.",
    },
    generate,
  };
}

function answerJson(request: AssistantModelRequest): string {
  const chunk = request.chunks[0];
  return JSON.stringify({
    version: 1,
    kind: request.kind,
    text:
      request.kind === "summary"
        ? "The report covers revenue and a new factory."
        : "Revenue is stated in the report.",
    citations: [{ chunkId: chunk.id, quote: "Revenue was" }],
  });
}

describe("assistantSessionReducer", () => {
  it("clears old document data when processing starts", () => {
    const populated = assistantSessionReducer(INITIAL_ASSISTANT_SESSION, {
      type: "processing-done",
      result,
    });
    const started = assistantSessionReducer(populated, {
      type: "start-processing",
    });

    expect(started.phase).toBe("processing");
    expect(started.chunks).toEqual([]);
    expect(started.sources).toEqual([]);
    expect(started.turns).toEqual([]);
    expect(started.progress).toEqual({
      stage: "inspecting",
      percent: 0,
      label: "Preparing documents",
    });
  });

  it("stores bounded ingestion output and returns to idle", () => {
    const state = assistantSessionReducer(INITIAL_ASSISTANT_SESSION, {
      type: "processing-done",
      result,
    });

    expect(state.phase).toBe("idle");
    expect(state.chunks).toHaveLength(2);
    expect(state.sources).toHaveLength(1);
    expect(state.stats?.chunkCount).toBe(2);
    expect(state.progress).toBeNull();
    expect(state.notice).toBeNull();
  });

  it("explains when processing produced no usable text", () => {
    const state = assistantSessionReducer(INITIAL_ASSISTANT_SESSION, {
      type: "processing-done",
      result: {
        ...result,
        chunks: [],
        stats: { ...stats, chunkCount: 0, chars: 0 },
      },
    });

    expect(state.notice).toContain("No usable text");
  });

  it("appends an answer turn and leaves the session idle", () => {
    let state = assistantSessionReducer(INITIAL_ASSISTANT_SESSION, {
      type: "start-answering",
      kind: "answer",
      request: "revenue",
    });
    expect(state.phase).toBe("answering");
    expect(state.pendingKind).toBe("answer");

    state = assistantSessionReducer(state, {
      type: "turn",
      turn: {
        id: "turn-1",
        kind: "answer",
        request: "revenue",
        truncated: false,
        answer: {
          version: 1,
          kind: "answer",
          text: "Revenue was 4.2 million euros.",
          citations: [],
        },
      },
    });

    expect(state.phase).toBe("idle");
    expect(state.pendingKind).toBeNull();
    expect(state.turns).toHaveLength(1);
    expect(state.progress).toBeNull();
  });

  it("keeps a failure visible without losing the document context", () => {
    const processed = assistantSessionReducer(INITIAL_ASSISTANT_SESSION, {
      type: "processing-done",
      result,
    });
    const failed = assistantSessionReducer(processed, {
      type: "fail",
      message: "The local model stopped.",
    });

    expect(failed.phase).toBe("idle");
    expect(failed.chunks).toEqual(chunks);
    expect(failed.error).toBe("The local model stopped.");
  });

  it("reports cancellation and drops partial progress", () => {
    const processing = assistantSessionReducer(INITIAL_ASSISTANT_SESSION, {
      type: "start-processing",
    });
    const cancelled = assistantSessionReducer(processing, { type: "cancel" });

    expect(cancelled.phase).toBe("idle");
    expect(cancelled.progress).toBeNull();
    expect(cancelled.notice).toContain("Cancelled");
  });

  it("clear releases chunks, sources, issues and history", () => {
    let state = assistantSessionReducer(INITIAL_ASSISTANT_SESSION, {
      type: "processing-done",
      result,
    });
    state = assistantSessionReducer(state, {
      type: "turn",
      turn: {
        id: "turn-1",
        kind: "insufficient",
        request: "volcano",
        message: "No evidence.",
      },
    });
    const cleared = assistantSessionReducer(state, { type: "clear" });

    expect(cleared).toEqual(INITIAL_ASSISTANT_SESSION);
    expect(hasSessionData(cleared)).toBe(false);
  });
});

describe("assistantIngestionProgress", () => {
  it("labels progress with the display name, never the path", () => {
    const progress: IngestionProgress = {
      sourceId: "f1",
      sourceIndex: 0,
      sourceCount: 2,
      stage: "ocr",
      percent: 42,
    };
    const mapped = assistantIngestionProgress(
      progress,
      new Map([["f1", "report.pdf"]]),
    );

    expect(mapped.stage).toBe("ocr");
    expect(mapped.percent).toBe(42);
    expect(mapped.label).toContain("report.pdf");
    expect(mapped.label).not.toContain("C:\\");
  });
});

describe("runGroundedAnswer", () => {
  it("passes only retrieved chunks to the model and returns verified citations", async () => {
    const generate = vi.fn(
      async (request: AssistantModelRequest, _signal: AbortSignal) =>
        answerJson(request),
    );
    const turn = await runGroundedAnswer(
      "turn-1",
      "revenue",
      chunks,
      readyModel(generate),
      new AbortController().signal,
    );

    expect(turn.kind).toBe("answer");
    if (turn.kind !== "answer") throw new Error("expected answer turn");
    expect(turn.request).toBe("revenue");
    expect(turn.answer.citations[0]?.sourceId).toBe("f1");
    expect(turn.answer.citations[0]?.page).toBe(1);
    expect(turn.answer.citations[0]?.quote).toBe("Revenue was");
    expect(generate).toHaveBeenCalledTimes(1);
    expect(generate.mock.calls[0][0].chunks.map((chunk) => chunk.id)).toEqual([
      "c1",
    ]);
  });

  it("does not call the model when retrieval has no evidence", async () => {
    const generate = vi.fn(
      async (request: AssistantModelRequest) => answerJson(request),
    );
    const turn = await runGroundedAnswer(
      "turn-1",
      "volcano",
      chunks,
      readyModel(generate),
      new AbortController().signal,
    );

    expect(turn.kind).toBe("insufficient");
    expect(generate).not.toHaveBeenCalled();
  });

  it("discards an unverifiable model answer", async () => {
    const turn = await runGroundedAnswer(
      "turn-1",
      "revenue",
      chunks,
      readyModel(async () =>
        JSON.stringify({
          version: 1,
          kind: "answer",
          text: "Revenue was 9 million.",
          citations: [{ chunkId: "c1", quote: "Revenue was 9 million." }],
        }),
      ),
      new AbortController().signal,
    );

    expect(turn.kind).toBe("error");
    if (turn.kind !== "error") throw new Error("expected error turn");
    expect(turn.message).toContain("could not be verified");
  });

  it("returns an honest message for invalid JSON", async () => {
    const turn = await runGroundedAnswer(
      "turn-1",
      "revenue",
      chunks,
      readyModel(async () => "not json"),
      new AbortController().signal,
    );

    expect(turn.kind).toBe("error");
    if (turn.kind !== "error") throw new Error("expected error turn");
    expect(turn.message).toContain("invalid answer format");
  });

  it("does not call the model after cancellation", async () => {
    const controller = new AbortController();
    controller.abort();
    const generate = vi.fn(
      async (request: AssistantModelRequest) => answerJson(request),
    );

    await expect(
      runGroundedAnswer(
        "turn-1",
        "revenue",
        chunks,
        readyModel(generate),
        controller.signal,
      ),
    ).rejects.toMatchObject({ name: "AbortError" });
    expect(generate).not.toHaveBeenCalled();
  });

  it("propagates model failures to the session hook", async () => {
    await expect(
      runGroundedAnswer(
        "turn-1",
        "revenue",
        chunks,
        readyModel(async () => {
          throw new Error("backend stopped");
        }),
        new AbortController().signal,
      ),
    ).rejects.toThrow("backend stopped");
  });
});

describe("runGroundedSummary", () => {
  it("requests a summary with sampled context and validates the candidate", async () => {
    const generate = vi.fn(
      async (request: AssistantModelRequest, _signal: AbortSignal) =>
        answerJson(request),
    );
    const turn = await runGroundedSummary(
      "turn-1",
      chunks,
      readyModel(generate),
      new AbortController().signal,
    );

    expect(turn.kind).toBe("summary");
    if (turn.kind !== "summary") throw new Error("expected summary turn");
    expect(turn.request).toBeUndefined();
    expect(turn.answer.kind).toBe("summary");
    expect(turn.answer.citations).toHaveLength(1);
    expect(generate.mock.calls[0][0].kind).toBe("summary");
    expect(generate.mock.calls[0][0].query).toBe("");
  });

  it("does not call the model for an empty document context", async () => {
    const generate = vi.fn(
      async (request: AssistantModelRequest) => answerJson(request),
    );
    const turn = await runGroundedSummary(
      "turn-1",
      [],
      readyModel(generate),
      new AbortController().signal,
    );

    expect(turn.kind).toBe("insufficient");
    expect(generate).not.toHaveBeenCalled();
  });
});

describe("session source helpers", () => {
  it("resolves a citation to the local source metadata", () => {
    expect(citationSource(result.sources, "f1")?.path).toBe(
      "C:\\private\\report.pdf",
    );
    expect(citationSource(result.sources, "missing")).toBeUndefined();
  });

  it("detects when the workspace no longer matches the processed sources", () => {
    expect(isSessionStale([{ uid: "f1" }], result.sources)).toBe(false);
    expect(isSessionStale([], result.sources)).toBe(true);
    expect(isSessionStale([{ uid: "f1" }, { uid: "f2" }], result.sources)).toBe(
      true,
    );
  });
});

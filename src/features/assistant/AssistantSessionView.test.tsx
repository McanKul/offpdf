import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { Citation } from "@/lib/assistant/citations";
import type { AssistantModelStatus } from "@/lib/assistant/model";
import {
  INITIAL_ASSISTANT_SESSION,
  type AssistantSessionState,
  type AssistantTurn,
} from "@/lib/assistant/session";
import type { WorkspaceFile } from "@/lib/types";
import { AssistantSessionView } from "./AssistantSessionView";

const READY: AssistantModelStatus = {
  state: "ready",
  backendId: "fake-v1",
  title: "Local model ready",
  detail: "The model runs on this device.",
};

const MISSING: AssistantModelStatus = {
  state: "not-installed",
  backendId: "none",
  title: "No local model installed",
  detail:
    "The private assistant is optional. It stays off until a local model is available, and no document text has been read.",
};

const FILES: WorkspaceFile[] = [
  {
    uid: "f1",
    path: "C:\\private\\report.pdf",
    name: "report.pdf",
    sizeBytes: 1000,
    pageCount: 2,
    isValidPdf: true,
  },
];

const SOURCES = [
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
];

const CITATION: Citation = {
  chunkId: "c1",
  sourceId: "f1",
  sourceName: "report.pdf",
  page: 1,
  quote: "Revenue was",
};

const ANSWER_TURN: AssistantTurn = {
  id: "turn-1",
  kind: "answer",
  request: "What was the revenue?",
  truncated: false,
  answer: {
    version: 1,
    kind: "answer",
    text: "Revenue was 4.2 million euros.",
    citations: [CITATION],
  },
};

function renderView(
  overrides: Partial<ComponentProps<typeof AssistantSessionView>> = {},
): string {
  return renderToStaticMarkup(
    <AssistantSessionView
      modelStatus={READY}
      session={INITIAL_ASSISTANT_SESSION}
      files={[]}
      documents={<div>Workspace document picker</div>}
      ocrEnabled={false}
      ocrLanguage=""
      ocrLoading={false}
      ocrAvailable={false}
      ocrLanguages={[]}
      ocrError={null}
      query=""
      onQueryChange={() => {}}
      onOcrEnabledChange={() => {}}
      onOcrLanguageChange={() => {}}
      onProcess={() => {}}
      onAsk={() => {}}
      onSummarize={() => {}}
      onCancel={() => {}}
      onOpenClear={() => {}}
      onOpenCitation={() => {}}
      {...overrides}
    />,
  );
}

function buttonMarkup(markup: string, label: string): string {
  return (
    (markup.match(/<button[^>]*>.*?<\/button>/gs) ?? []).find((button) =>
      button.includes(label),
    ) ?? ""
  );
}

function session(
  overrides: Partial<AssistantSessionState>,
): AssistantSessionState {
  return { ...INITIAL_ASSISTANT_SESSION, ...overrides };
}

describe("AssistantSessionView", () => {
  it("keeps the assistant clearly optional when no local model exists", () => {
    const markup = renderView({ modelStatus: MISSING });

    expect(markup).toContain("No local model installed");
    expect(markup).toContain("Optional");
    expect(markup).toContain("no document text has been read");
    expect(markup).toContain(
      "The assistant is off while no local model is installed",
    );
    expect(markup).toContain("Nothing is synced or uploaded");
    expect(buttonMarkup(markup, "Process documents")).toContain("disabled");
    expect(buttonMarkup(markup, "Clear session")).toContain("disabled");
  });

  it("explains the empty ready state and keeps processing disabled without files", () => {
    const markup = renderView();

    expect(markup).toContain("Workspace document picker");
    expect(markup).toContain("Process at least one document first");
    expect(buttonMarkup(markup, "Process documents")).toContain("disabled");
    expect(markup).toContain("No answers yet");
  });

  it("renders determinate processing progress and a cancel control", () => {
    const markup = renderView({
      session: session({
        phase: "processing",
        progress: {
          stage: "ocr",
          percent: 42,
          label: "Running OCR on report.pdf (1 of 1)",
        },
      }),
    });

    expect(markup).toContain('aria-busy="true"');
    expect(markup).toContain("Running OCR on report.pdf (1 of 1)");
    expect(markup).toContain("42%");
    expect(markup).toContain('aria-valuenow="42"');
    expect(markup).toContain("Reading only the documents you selected");
    expect(buttonMarkup(markup, "Cancel")).not.toContain("disabled");
    expect(markup).toContain("Processing...");
  });

  it("renders an answering state with an indeterminate progress bar", () => {
    const markup = renderView({
      session: session({
        phase: "answering",
        pendingKind: "answer",
        progress: {
          stage: "thinking",
          percent: null,
          label: "Reading the selected pages",
        },
      }),
    });

    expect(markup).toContain("Thinking");
    expect(markup).toContain("Reading the selected pages");
    expect(markup).toContain("progress--indeterminate");
    expect(markup).toContain('aria-busy="true"');
  });

  it("renders a request error as a visible alert", () => {
    const markup = renderView({
      session: session({ error: "The local model stopped." }),
    });

    expect(markup).toContain("Request failed");
    expect(markup).toContain("The local model stopped.");
    expect(markup).toContain("alert--danger");
  });

  it("renders a grounded answer with an accessible citation button", () => {
    const markup = renderView({
      files: FILES,
      session: session({
        sources: SOURCES,
        chunks: [
          {
            id: "c1",
            sourceId: "f1",
            sourceName: "report.pdf",
            page: 1,
            text: "Revenue was 4.2 million euros.",
          },
        ],
        turns: [ANSWER_TURN],
        stats: {
          sourceCount: 1,
          pageCount: 2,
          nativePages: 2,
          ocrPages: 0,
          emptyPages: 0,
          chunkCount: 1,
          chars: 30,
          truncated: false,
        },
      }),
    });

    expect(markup).toContain('role="log"');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain("Revenue was 4.2 million euros.");
    expect(markup).toContain(
      "Open report.pdf, p. 1 in the page viewer",
    );
    expect(buttonMarkup(markup, "Clear session")).not.toContain("disabled");
  });

  it("warns when the workspace changed after processing", () => {
    const markup = renderView({
      files: [
        ...FILES,
        {
          uid: "f2",
          path: "C:\\private\\second.pdf",
          name: "second.pdf",
          sizeBytes: 1000,
          pageCount: 1,
          isValidPdf: true,
        },
      ],
      session: session({ sources: SOURCES }),
    });

    expect(markup).toContain("Workspace changed");
    expect(markup).toContain("Process again");
  });

  it("shows an unavailable OCR state only after OCR is requested", () => {
    const markup = renderView({
      files: FILES,
      ocrEnabled: true,
      ocrAvailable: false,
    });

    expect(markup).toContain("OCR is not available");
    expect(buttonMarkup(markup, "Process documents")).toContain("disabled");
  });

  it("keeps keyboard and focus affordances in the markup", () => {
    const markup = renderView({
      session: session({
        chunks: [
          {
            id: "c1",
            sourceId: "f1",
            sourceName: "report.pdf",
            page: 1,
            text: "Revenue was 4.2 million euros.",
          },
        ],
      }),
    });

    expect(markup).toContain('aria-keyshortcuts="Control+Enter Meta+Enter"');
    expect(markup).toContain('tabindex="-1"');
    expect(markup).toContain("Ctrl+Enter asks");
  });
});

import { describe, expect, it } from "vitest";
import type { IngestionIssue } from "@/lib/assistant/ingestion";
import type { AssistantModelStatus } from "@/lib/assistant/model";
import {
  canProcessDocuments,
  canRunAssistant,
  clearSessionFacts,
  clearSessionKeeps,
  clearSessionSummary,
  documentCountLabel,
  historyEmptyText,
  ingestionSummary,
  issueRows,
  modelStatusView,
  turnRequestLabel,
} from "./assistantView";
import { INITIAL_ASSISTANT_SESSION } from "@/lib/assistant/session";

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
  detail: "The assistant is optional.",
};

describe("modelStatusView", () => {
  it("marks a ready model as usable", () => {
    const view = modelStatusView(READY);
    expect(view.ready).toBe(true);
    expect(view.badgeLabel).toBe("Ready");
    expect(view.badgeVariant).toBe("success");
  });

  it("keeps a missing model clearly optional", () => {
    const view = modelStatusView(MISSING);
    expect(view.ready).toBe(false);
    expect(view.badgeLabel).toBe("Optional");
    expect(view.badgeVariant).toBe("warning");
    expect(view.alertVariant).toBe("warning");
  });

  it("shows an unsupported or failed model as an error state", () => {
    expect(
      modelStatusView({ ...READY, state: "unsupported" }).badgeVariant,
    ).toBe("danger");
    expect(modelStatusView({ ...READY, state: "error" }).badgeLabel).toBe(
      "Unavailable",
    );
  });
});

describe("ingestion view helpers", () => {
  it("formats bounded document statistics", () => {
    expect(
      ingestionSummary({
        sourceCount: 2,
        pageCount: 120,
        nativePages: 119,
        ocrPages: 1,
        emptyPages: 1,
        chunkCount: 14,
        chars: 2000,
        truncated: false,
      }),
    ).toBe("2 documents, 120 pages, 14 search chunks, 1 OCR");
  });

  it("marks capped sessions", () => {
    expect(
      ingestionSummary({
        sourceCount: 1,
        pageCount: 1001,
        nativePages: 1000,
        ocrPages: 0,
        emptyPages: 1,
        chunkCount: 1,
        chars: 10,
        truncated: true,
      }),
    ).toContain("(limits applied)");
  });

  it("turns structured issues into readable rows with page ranges", () => {
    const issues: IngestionIssue[] = [
      {
        sourceId: "f1",
        sourceName: "scan.pdf",
        code: "OCR_REQUIRED",
        message: "Some pages have no usable text layer.",
        pages: [2, 3, 4],
      },
      {
        code: "TOO_MANY_SOURCES",
        message: "Only the first 20 documents can be ingested at once.",
      },
    ];

    expect(issueRows(issues)).toEqual([
      {
        title: "Scanned pages need OCR",
        where: "scan.pdf, pp. 2\u20134",
        message: "Some pages have no usable text layer.",
      },
      {
        title: "Document limit",
        where: null,
        message: "Only the first 20 documents can be ingested at once.",
      },
    ]);
  });

  it("describes empty history honestly for each state", () => {
    expect(historyEmptyText(false, 0)).toContain("no local model");
    expect(historyEmptyText(true, 0)).toContain("Process the selected documents");
    expect(historyEmptyText(true, 3)).toContain("memory only");
  });
});

describe("assistant action availability", () => {
  it("only runs questions when idle, with a model and processed chunks", () => {
    expect(canRunAssistant("idle", true, 1)).toBe(true);
    expect(canRunAssistant("processing", true, 1)).toBe(false);
    expect(canRunAssistant("idle", false, 1)).toBe(false);
    expect(canRunAssistant("idle", true, 0)).toBe(false);
  });

  it("requires a valid OCR choice when OCR is enabled", () => {
    const idle = { enabled: false, loading: false, available: true, language: "" };
    expect(canProcessDocuments("idle", true, 1, idle)).toBe(true);
    expect(
      canProcessDocuments("idle", true, 1, {
        ...idle,
        enabled: true,
        available: false,
      }),
    ).toBe(false);
    expect(
      canProcessDocuments("idle", true, 1, {
        ...idle,
        enabled: true,
        language: "",
      }),
    ).toBe(false);
    expect(
      canProcessDocuments("idle", true, 1, {
        ...idle,
        enabled: true,
        language: "eng",
      }),
    ).toBe(true);
  });
});

describe("session copy", () => {
  it("states exactly what clear removes and what it keeps", () => {
    expect(clearSessionFacts()).toContain("Extracted page text and search chunks");
    expect(clearSessionFacts()).toContain(
      "Every question, summary, answer and error shown here",
    );
    expect(clearSessionKeeps()).toContain(
      "Your PDF files are not changed or deleted.",
    );
    expect(clearSessionKeeps()).toContain(
      "The local model stays installed and loaded.",
    );
    expect(
      clearSessionSummary({
        ...INITIAL_ASSISTANT_SESSION,
        sources: [
          {
            id: "f1",
            path: "C:\\private\\report.pdf",
            name: "report.pdf",
            pageCount: 2,
            consideredPages: 2,
            nativePages: 1,
            ocrPages: 1,
            emptyPages: 1,
          },
        ],
        chunks: [],
        turns: [],
        stats: {
          sourceCount: 1,
          pageCount: 2,
          nativePages: 1,
          ocrPages: 1,
          emptyPages: 1,
          chunkCount: 0,
          chars: 0,
          truncated: false,
        },
      }),
    ).toBe(
      "This removes 1 processed document, 0 search chunks, 1 OCR page and 0 history turns from memory:",
    );
  });

  it("labels summary and question turns", () => {
    expect(turnRequestLabel("summary")).toBe("Summary");
    expect(turnRequestLabel("answer", "What changed?")).toBe("What changed?");
    expect(turnRequestLabel("error")).toBe("Request failed");
  });

  it("distinguishes selected from processed documents", () => {
    expect(documentCountLabel(3, 0)).toBe("3 selected");
    expect(documentCountLabel(3, 2)).toBe("2 processed");
  });
});

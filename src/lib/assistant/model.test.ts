import { describe, expect, it } from "vitest";
import {
  buildAssistantPrompt,
  getAssistantModel,
  isAssistantModelReady,
  NO_LOCAL_MODEL,
  type AssistantModel,
} from "./model";
import type { RetrievedChunk } from "./retrieval";

const chunk: RetrievedChunk = {
  id: "c1",
  sourceId: "f1",
  sourceName: "report.pdf",
  page: 1,
  text: "Revenue was 4.2 million euros.",
  score: 1,
  matchedTerms: ["revenue"],
};

describe("assistant model contract", () => {
  it("ships an explicitly unavailable default with no network fallback", async () => {
    expect(getAssistantModel().status.state).toBe("not-installed");
    expect(isAssistantModelReady(NO_LOCAL_MODEL)).toBe(false);
    await expect(
      NO_LOCAL_MODEL.generate(
        {
          kind: "answer",
          query: "revenue",
          chunks: [chunk],
          truncated: false,
        },
        new AbortController().signal,
      ),
    ).rejects.toMatchObject({ name: "AssistantModelUnavailableError" });
  });

  it("recognizes a ready adapter", () => {
    const ready: AssistantModel = {
      id: "fake",
      status: {
        state: "ready",
        backendId: "fake-v1",
        title: "Ready",
        detail: "On device.",
      },
      generate: async () => "",
    };
    expect(isAssistantModelReady(ready)).toBe(true);
  });

  it("builds a bounded prompt with only chunk metadata and the JSON contract", () => {
    const prompt = buildAssistantPrompt({
      kind: "answer",
      query: "revenue",
      chunks: [chunk],
      truncated: true,
    });

    expect(prompt).toContain("[c1] report.pdf, page 1");
    expect(prompt).toContain("Revenue was 4.2 million euros.");
    expect(prompt).toContain('"kind":"answer"|"summary"|"insufficient"');
    expect(prompt).toContain("insufficient");
    expect(prompt).toContain("bounded");
    expect(prompt).not.toContain("C:\\");
  });
});

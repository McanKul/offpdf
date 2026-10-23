import { describe, expect, it } from "vitest";
import {
  INGESTION_LIMITS,
  IngestionCancelledError,
  hasUsableText,
  ingestPdfSources,
  normalizePageText,
  sourceBasename,
  splitPageText,
  type ExtractedText,
  type IngestionFileInfo,
  type IngestionRuntime,
  type IngestionSource,
} from "./ingestion";
import ingestionJson from "./fixtures/ingestion.json";

interface IngestionFixture {
  native: { name: string; pages: string[] };
  scanned: { name: string; pages: string[] };
  mixed: { name: string; pages: string[] };
  ocr: { scanned: string[]; mixed: string[] };
  longPage: string;
}

const fixture = ingestionJson as unknown as IngestionFixture;
type OcrCall = Parameters<IngestionRuntime["ocrText"]>[0];

interface RuntimeOverrides {
  info?:
    | IngestionFileInfo
    | ((path: string) => IngestionFileInfo | Promise<IngestionFileInfo>);
  extract?:
    | ExtractedText
    | ((path: string) => ExtractedText | Promise<ExtractedText>);
  ocr?:
    | ExtractedText
    | ((input: OcrCall) => ExtractedText | Promise<ExtractedText>);
}

function makeRuntime(overrides: RuntimeOverrides = {}) {
  const calls = {
    inspected: [] as string[],
    extracted: [] as string[],
    ocr: [] as OcrCall[],
  };
  const runtime: IngestionRuntime = {
    inspect: async (path) => {
      calls.inspected.push(path);
      if (typeof overrides.info === "function") return overrides.info(path);
      return (
        overrides.info ?? {
          sizeBytes: 1_000,
          pageCount: 1,
          isValidPdf: true,
        }
      );
    },
    extractText: async (path) => {
      calls.extracted.push(path);
      if (typeof overrides.extract === "function") {
        return overrides.extract(path);
      }
      return overrides.extract ?? { pages: [], truncated: false };
    },
    ocrText: async (input) => {
      calls.ocr.push(input);
      if (typeof overrides.ocr === "function") return overrides.ocr(input);
      return overrides.ocr ?? { pages: [], truncated: false };
    },
  };
  return { runtime, calls };
}

function source(path: string, id?: string, name?: string): IngestionSource {
  return { path, id, name };
}

const windowsPath =
  "C:\\Users\\Alice\\Private Archive\\Annual Report 2025.pdf";

describe("ingestPdfSources native text", () => {
  it("chunks native pages with stable source and page metadata", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 10_000, pageCount: 3, isValidPdf: true },
      extract: { pages: fixture.native.pages, truncated: false },
    });

    const result = await ingestPdfSources(
      [source(windowsPath, "f1", fixture.native.name)],
      runtime,
    );

    expect(calls.inspected).toEqual([windowsPath]);
    expect(calls.extracted).toEqual([windowsPath]);
    expect(calls.ocr).toEqual([]);
    expect(result.issues).toEqual([]);
    expect(result.chunks.map((chunk) => chunk.page)).toEqual([1, 2, 3]);
    expect(result.chunks.map((chunk) => chunk.sourceId)).toEqual([
      "f1",
      "f1",
      "f1",
    ]);
    expect(result.chunks.map((chunk) => chunk.id)).toEqual([
      "f1:p1:c1",
      "f1:p2:c1",
      "f1:p3:c1",
    ]);
    expect(result.chunks.every((chunk) => chunk.sourceName === fixture.native.name)).toBe(
      true,
    );
    expect(JSON.stringify(result.chunks)).not.toContain("Alice");
    expect(result.sources).toEqual([
      {
        id: "f1",
        path: windowsPath,
        name: fixture.native.name,
        pageCount: 3,
        consideredPages: 3,
        nativePages: 3,
        ocrPages: 0,
        emptyPages: 0,
      },
    ]);
    expect(result.stats).toMatchObject({
      sourceCount: 1,
      pageCount: 3,
      nativePages: 3,
      ocrPages: 0,
      emptyPages: 0,
      chunkCount: 3,
      truncated: false,
    });
  });

  it("only reads the explicitly supplied paths, never a containing folder", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: ["A sufficiently useful native paragraph."], truncated: false },
    });

    await ingestPdfSources(
      [source(windowsPath, "f1"), source("/tmp/other.pdf", "f2")],
      runtime,
    );

    expect(calls.inspected).toEqual([windowsPath, "/tmp/other.pdf"]);
    expect(calls.extracted).toEqual([windowsPath, "/tmp/other.pdf"]);
    expect(calls.inspected).not.toContain("C:\\Users\\Alice\\Private Archive");
  });

  it("is deterministic for identical input", async () => {
    const overrides: RuntimeOverrides = {
      info: { sizeBytes: 100, pageCount: 2, isValidPdf: true },
      extract: { pages: fixture.native.pages.slice(0, 2), truncated: false },
    };
    const first = await ingestPdfSources(
      [source("/tmp/report.pdf", "f1")],
      makeRuntime(overrides).runtime,
    );
    const second = await ingestPdfSources(
      [source("/tmp/report.pdf", "f1")],
      makeRuntime(overrides).runtime,
    );
    expect(second).toEqual(first);
  });

  it("keeps page boundaries when a page needs several chunks", async () => {
    const { runtime } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: [fixture.longPage], truncated: false },
    });

    const result = await ingestPdfSources([source("/tmp/long.pdf", "f1")], runtime, {
      limits: { maxChunkChars: 200 },
    });

    expect(result.chunks.length).toBeGreaterThan(1);
    expect(result.chunks.every((chunk) => chunk.page === 1)).toBe(true);
    expect(result.chunks.every((chunk) => chunk.text.length <= 200)).toBe(true);
    expect(new Set(result.chunks.map((chunk) => chunk.id)).size).toBe(
      result.chunks.length,
    );
  });
});

describe("ingestPdfSources OCR", () => {
  it("reports scanned pages and does not OCR implicitly", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 3, isValidPdf: true },
      extract: { pages: fixture.scanned.pages, truncated: false },
    });

    const result = await ingestPdfSources(
      [source("/tmp/scanned.pdf", "f1", fixture.scanned.name)],
      runtime,
    );

    expect(calls.ocr).toEqual([]);
    expect(result.chunks).toEqual([]);
    expect(result.issues).toHaveLength(1);
    expect(result.issues[0]).toMatchObject({
      sourceId: "f1",
      code: "OCR_REQUIRED",
      pages: [1, 2, 3],
    });
    expect(result.stats.emptyPages).toBe(3);
  });

  it("OCRs all unusable pages when explicitly enabled", async () => {
    const stages: string[] = [];
    const percents: number[] = [];
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 3, isValidPdf: true },
      extract: { pages: fixture.scanned.pages, truncated: false },
      ocr: { pages: fixture.ocr.scanned, truncated: false },
    });

    const result = await ingestPdfSources(
      [source("/tmp/scanned.pdf", "f1", fixture.scanned.name)],
      runtime,
      {
        ocr: { enabled: true, language: "eng" },
        onProgress: (progress) => {
          stages.push(progress.stage);
          percents.push(progress.percent);
        },
      },
    );

    expect(calls.ocr).toHaveLength(1);
    expect(calls.ocr[0].pages).toEqual([1, 2, 3]);
    expect(calls.ocr[0].language).toBe("eng");
    expect(result.chunks.map((chunk) => chunk.page)).toEqual([1, 2, 3]);
    expect(result.sources[0]).toMatchObject({
      nativePages: 0,
      ocrPages: 3,
      emptyPages: 0,
    });
    expect(result.issues).toEqual([]);
    expect(stages).toContain("ocr");
    expect(stages[stages.length - 1]).toBe("done");
    expect(percents.every((percent) => percent >= 0 && percent <= 100)).toBe(
      true,
    );
    expect(percents[percents.length - 1]).toBe(100);
  });

  it("OCRs only pages without usable native text in a mixed document", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 4, isValidPdf: true },
      extract: { pages: fixture.mixed.pages, truncated: false },
      ocr: { pages: fixture.ocr.mixed, truncated: false },
    });

    const result = await ingestPdfSources(
      [source("/tmp/mixed.pdf", "f1", fixture.mixed.name)],
      runtime,
      { ocr: { enabled: true, language: "eng" } },
    );

    expect(calls.ocr).toHaveLength(1);
    expect(calls.ocr[0].pages).toEqual([2, 4]);
    expect(result.chunks.map((chunk) => chunk.page)).toEqual([1, 2, 3, 4]);
    expect(result.sources[0]).toMatchObject({
      nativePages: 2,
      ocrPages: 2,
      emptyPages: 0,
    });
    expect(result.issues).toEqual([]);
  });

  it("honours an explicit OCR page subset", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 4, isValidPdf: true },
      extract: { pages: fixture.mixed.pages, truncated: false },
      ocr: { pages: ["OCR page four recovered the signature."], truncated: false },
    });

    const result = await ingestPdfSources(
      [source("/tmp/mixed.pdf", "f1", fixture.mixed.name)],
      runtime,
      { ocr: { enabled: true, language: "eng", pages: [4] } },
    );

    expect(calls.ocr[0].pages).toEqual([4]);
    expect(result.chunks.map((chunk) => chunk.page)).toEqual([1, 3, 4]);
    expect(result.stats.emptyPages).toBe(1);
  });

  it("does not replace native text with OCR when a page has both", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: ["Native words that are already usable."], truncated: false },
      ocr: { pages: ["OCR replacement should never be used."], truncated: false },
    });

    const result = await ingestPdfSources([source("/tmp/one.pdf", "f1")], runtime, {
      ocr: { enabled: true, language: "eng", pages: [1] },
    });

    expect(calls.ocr).toEqual([]);
    expect(result.chunks[0].text).toContain("Native words");
  });

  it("reports a missing OCR language", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: [""], truncated: false },
    });

    const result = await ingestPdfSources([source("/tmp/scan.pdf", "f1")], runtime, {
      ocr: { enabled: true },
    });

    expect(calls.ocr).toEqual([]);
    expect(result.issues[0]).toMatchObject({
      code: "OCR_LANGUAGE_REQUIRED",
      pages: [1],
    });
  });

  it("bounds OCR work and reports omitted pages", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 3, isValidPdf: true },
      extract: { pages: fixture.scanned.pages, truncated: false },
      ocr: { pages: fixture.ocr.scanned, truncated: false },
    });

    const result = await ingestPdfSources([source("/tmp/scan.pdf", "f1")], runtime, {
      ocr: { enabled: true, language: "eng", maxPages: 2 },
    });

    expect(calls.ocr[0].pages).toEqual([1, 2]);
    expect(result.issues).toContainEqual(
      expect.objectContaining({ code: "OCR_LIMIT", pages: [3] }),
    );
    expect(result.stats.truncated).toBe(true);
  });

  it("shares the OCR page budget across every source", async () => {
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 2, isValidPdf: true },
      extract: { pages: ["", ""], truncated: false },
      ocr: {
        pages: ["OCR page one.", "OCR page two."],
        truncated: false,
      },
    });

    const result = await ingestPdfSources(
      [source("/tmp/one.pdf", "f1"), source("/tmp/two.pdf", "f2")],
      runtime,
      {
        ocr: { enabled: true, language: "eng" },
        limits: { maxOcrPages: 2 },
      },
    );

    expect(calls.ocr).toHaveLength(1);
    expect(calls.ocr[0].pages).toEqual([1, 2]);
    expect(result.issues).toContainEqual(
      expect.objectContaining({
        sourceId: "f2",
        code: "OCR_LIMIT",
        pages: [1, 2],
      }),
    );
    expect(result.stats.ocrPages).toBe(2);
    expect(result.stats.emptyPages).toBe(2);
    expect(result.stats.truncated).toBe(true);
  });

  it("classifies OCR availability, failure and empty output", async () => {
    const missing = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: [""], truncated: false },
      ocr: () => {
        throw { code: "OCR_MISSING", message: "no tesseract" };
      },
    });
    const missingResult = await ingestPdfSources(
      [source("/tmp/scan.pdf", "f1")],
      missing.runtime,
      { ocr: { enabled: true, language: "eng" } },
    );
    expect(missingResult.issues[0].code).toBe("OCR_UNAVAILABLE");

    const failed = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: [""], truncated: false },
      ocr: () => {
        throw { code: "OCR_FAILED" };
      },
    });
    const failedResult = await ingestPdfSources(
      [source("/tmp/scan.pdf", "f1")],
      failed.runtime,
      { ocr: { enabled: true, language: "eng" } },
    );
    expect(failedResult.issues[0].code).toBe("OCR_FAILED");

    const empty = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: [""], truncated: false },
      ocr: { pages: ["   "], truncated: false },
    });
    const emptyResult = await ingestPdfSources(
      [source("/tmp/scan.pdf", "f1")],
      empty.runtime,
      { ocr: { enabled: true, language: "eng" } },
    );
    expect(emptyResult.issues[0]).toMatchObject({
      code: "OCR_EMPTY",
      pages: [1],
    });
    expect(emptyResult.chunks).toEqual([]);
  });
});

describe("ingestPdfSources failure and limit handling", () => {
  it("reports non-PDF, empty and unreadable documents", async () => {
    const unreadable = makeRuntime({
      info: { sizeBytes: 10, pageCount: null, isValidPdf: false },
    });
    const unreadableResult = await ingestPdfSources(
      [source("/tmp/broken.pdf", "f1")],
      unreadable.runtime,
    );
    expect(unreadableResult.issues[0].code).toBe("NOT_PDF");
    expect(unreadable.calls.extracted).toEqual([]);

    const empty = makeRuntime({
      info: { sizeBytes: 10, pageCount: 0, isValidPdf: true },
    });
    const emptyResult = await ingestPdfSources(
      [source("/tmp/empty.pdf", "f1")],
      empty.runtime,
    );
    expect(emptyResult.issues[0].code).toBe("EMPTY_PDF");

    const throws = makeRuntime({
      info: () => {
        throw new Error("cannot read");
      },
    });
    const throwsResult = await ingestPdfSources(
      [source("/tmp/encrypted.pdf", "f1")],
      throws.runtime,
    );
    expect(throwsResult.issues[0].code).toBe("UNREADABLE_PDF");
  });

  it("reports native extraction failure without dropping other sources", async () => {
    const { runtime } = makeRuntime({
      info: () => ({
        sizeBytes: 100,
        pageCount: 1,
        isValidPdf: true,
      }),
      extract: (path) => {
        if (path.includes("bad")) throw new Error("engine failed");
        return { pages: ["Useful text from the good document."], truncated: false };
      },
    });

    const result = await ingestPdfSources(
      [source("/tmp/bad.pdf", "f1"), source("/tmp/good.pdf", "f2")],
      runtime,
    );

    expect(result.issues).toContainEqual(
      expect.objectContaining({ sourceId: "f1", code: "EXTRACTION_FAILED" }),
    );
    expect(result.chunks.map((chunk) => chunk.sourceId)).toEqual(["f2"]);
  });

  it("bounds source size, pages, chunks and characters", async () => {
    const tooLarge = makeRuntime({
      info: { sizeBytes: INGESTION_LIMITS.maxSourceBytes + 1, pageCount: 1, isValidPdf: true },
    });
    const largeResult = await ingestPdfSources(
      [source("/tmp/large.pdf", "f1")],
      tooLarge.runtime,
    );
    expect(largeResult.issues[0].code).toBe("SOURCE_TOO_LARGE");
    expect(tooLarge.calls.extracted).toEqual([]);

    const pages = Array.from({ length: 5 }, (_, index) => `Useful page text ${index + 1}.`);
    const paged = makeRuntime({
      info: { sizeBytes: 100, pageCount: 5, isValidPdf: true },
      extract: { pages, truncated: false },
    });
    const pageResult = await ingestPdfSources([source("/tmp/five.pdf", "f1")], paged.runtime, {
      limits: { maxPagesPerSource: 3 },
    });
    expect(pageResult.sources[0].consideredPages).toBe(3);
    expect(pageResult.chunks.map((chunk) => chunk.page)).toEqual([1, 2, 3]);
    expect(pageResult.issues).toContainEqual(
      expect.objectContaining({ code: "PAGE_LIMIT" }),
    );
    expect(pageResult.stats.truncated).toBe(true);

    const multi = makeRuntime({
      info: () => ({
        sizeBytes: 100,
        pageCount: 2,
        isValidPdf: true,
      }),
      extract: { pages: ["Useful first page.", "Useful second page."], truncated: false },
    });
    const multiResult = await ingestPdfSources(
      [source("/tmp/one.pdf", "f1"), source("/tmp/two.pdf", "f2")],
      multi.runtime,
      { limits: { maxTotalPages: 3 } },
    );
    expect(multiResult.sources.map((item) => item.consideredPages)).toEqual([2, 1]);
    expect(multiResult.stats.pageCount).toBe(3);
    expect(multiResult.stats.truncated).toBe(true);

    const index = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: [fixture.longPage], truncated: false },
    });
    const indexResult = await ingestPdfSources([source("/tmp/long.pdf", "f1")], index.runtime, {
      limits: { maxIndexChars: 50, maxChunkChars: 200 },
    });
    expect(indexResult.chunks).toEqual([]);
    expect(indexResult.issues).toContainEqual(
      expect.objectContaining({ code: "INDEX_LIMIT" }),
    );
    expect(indexResult.stats.truncated).toBe(true);

    const counted = makeRuntime({
      info: { sizeBytes: 100, pageCount: 3, isValidPdf: true },
      extract: {
        pages: ["Useful page one.", "Useful page two.", "Useful page three."],
        truncated: false,
      },
    });
    const countedResult = await ingestPdfSources(
      [source("/tmp/three.pdf", "f1")],
      counted.runtime,
      { limits: { maxChunks: 1 } },
    );
    expect(countedResult.chunks).toHaveLength(1);
    expect(countedResult.stats.truncated).toBe(true);
  });

  it("caps the number of sources and rejects invalid or duplicate ids", async () => {
    const many = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: ["Useful text."], truncated: false },
    });
    const manyResult = await ingestPdfSources(
      [source("/tmp/one.pdf", "f1"), source("/tmp/two.pdf", "f2")],
      many.runtime,
      { limits: { maxSources: 1 } },
    );
    expect(manyResult.issues[0].code).toBe("TOO_MANY_SOURCES");
    expect(many.calls.inspected).toEqual(["/tmp/one.pdf"]);

    const invalid = makeRuntime();
    const invalidResult = await ingestPdfSources(
      [source("/tmp/one.pdf", "../secret")],
      invalid.runtime,
    );
    expect(invalidResult.issues[0].code).toBe("INVALID_SOURCE_ID");
    expect(invalid.calls.inspected).toEqual([]);

    const duplicate = makeRuntime({
      info: { sizeBytes: 100, pageCount: 1, isValidPdf: true },
      extract: { pages: ["Useful text."], truncated: false },
    });
    const duplicateResult = await ingestPdfSources(
      [source("/tmp/one.pdf", "f1"), source("/tmp/two.pdf", "f1")],
      duplicate.runtime,
    );
    expect(duplicateResult.issues[0].code).toBe("DUPLICATE_SOURCE_ID");
    expect(duplicate.calls.inspected).toEqual(["/tmp/one.pdf"]);
  });
});

describe("ingestPdfSources cancellation", () => {
  it("does not start when the signal is already aborted", async () => {
    const controller = new AbortController();
    controller.abort();
    const { runtime, calls } = makeRuntime();

    await expect(
      ingestPdfSources([source("/tmp/one.pdf", "f1")], runtime, {
        signal: controller.signal,
      }),
    ).rejects.toBeInstanceOf(IngestionCancelledError);
    expect(calls.inspected).toEqual([]);
  });

  it("surfaces cancellation during OCR and stops without partial chunks", async () => {
    const controller = new AbortController();
    const { runtime, calls } = makeRuntime({
      info: { sizeBytes: 100, pageCount: 2, isValidPdf: true },
      extract: { pages: ["", "Native page two is useful."], truncated: false },
      ocr: async () => {
        controller.abort();
        throw { code: "CANCELLED" };
      },
    });

    await expect(
      ingestPdfSources([source("/tmp/scan.pdf", "f1")], runtime, {
        signal: controller.signal,
        ocr: { enabled: true, language: "eng" },
      }),
    ).rejects.toBeInstanceOf(IngestionCancelledError);
    expect(calls.ocr).toHaveLength(1);
  });
});

describe("ingestion helpers", () => {
  it("normalizes page text without merging pages", () => {
    expect(normalizePageText("a\r\n\r\n\r\nb\u0000\u00a0c")).toBe("a\n\nb c");
  });

  it("treats page numbers and punctuation as unusable, titles as usable", () => {
    expect(hasUsableText("12")).toBe(false);
    expect(hasUsableText("  \n\t ")).toBe(false);
    expect(hasUsableText("Chapter 1")).toBe(true);
    expect(hasUsableText("Introduction")).toBe(true);
  });

  it("returns a basename without directory or control characters", () => {
    expect(sourceBasename(windowsPath)).toBe("Annual Report 2025.pdf");
    expect(sourceBasename("/tmp/private/report.pdf")).toBe("report.pdf");
    expect(sourceBasename("C:\\tmp\\bad\u0000name.pdf")).toBe("badname.pdf");
    expect(sourceBasename("/tmp/folder/")).toBe("document.pdf");
  });

  it("splits a long page at word boundaries without empty chunks", () => {
    const chunks = splitPageText(fixture.longPage, 200);
    expect(chunks.length).toBeGreaterThan(1);
    expect(chunks.every((chunk) => chunk.length > 0 && chunk.length <= 200)).toBe(
      true,
    );
    expect(chunks[0]).toContain("Local-first");
    expect(chunks[chunks.length - 1]).toContain("contract");
  });
});

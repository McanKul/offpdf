import { beforeEach, describe, expect, it, vi } from "vitest";

const commands = vi.hoisted(() => ({
  assistantPdfText: vi.fn(),
  assistantOcrText: vi.fn(),
  cancelJob: vi.fn(),
  getFileInfo: vi.fn(),
  newJobId: vi.fn(),
  onJobUpdate: vi.fn(),
}));

vi.mock("@/lib/tauriCommands", () => commands);

import type { AssistantText } from "@/lib/types";
import { createTauriIngestionRuntime } from "./ingestionTauri";

const unlisten = vi.fn();

beforeEach(() => {
  vi.resetAllMocks();
  commands.newJobId.mockReturnValue("job-1");
  commands.getFileInfo.mockResolvedValue({
    path: "/tmp/report.pdf",
    name: "report.pdf",
    sizeBytes: 1_000,
    pageCount: 2,
    isValidPdf: true,
  });
  commands.assistantPdfText.mockResolvedValue({
    pages: ["native page one", "native page two"],
    truncated: false,
  });
  commands.assistantOcrText.mockResolvedValue({
    pages: ["ocr page one"],
    truncated: false,
  });
  commands.cancelJob.mockResolvedValue(undefined);
  commands.onJobUpdate.mockResolvedValue(unlisten);
});

describe("createTauriIngestionRuntime", () => {
  it("maps file info and native text through IPC", async () => {
    const runtime = createTauriIngestionRuntime();

    await expect(runtime.inspect("/tmp/report.pdf")).resolves.toEqual({
      sizeBytes: 1_000,
      pageCount: 2,
      isValidPdf: true,
    });
    await expect(runtime.extractText("/tmp/report.pdf")).resolves.toEqual({
      pages: ["native page one", "native page two"],
      truncated: false,
    });

    expect(commands.getFileInfo).toHaveBeenCalledWith("/tmp/report.pdf");
    expect(commands.assistantPdfText).toHaveBeenCalledWith("/tmp/report.pdf");
  });

  it("forwards OCR progress for the job and cleans up the listener", async () => {
    const runtime = createTauriIngestionRuntime();
    const controller = new AbortController();
    const onProgress = vi.fn();

    const result = await runtime.ocrText({
      path: "/tmp/report.pdf",
      pages: [2, 4],
      language: "eng",
      signal: controller.signal,
      onProgress,
    });

    expect(commands.assistantOcrText).toHaveBeenCalledWith(
      "job-1",
      "/tmp/report.pdf",
      [2, 4],
      "eng",
    );
    expect(commands.onJobUpdate).toHaveBeenCalledWith(expect.any(Function), "job-1");
    expect(result).toEqual({ pages: ["ocr page one"], truncated: false });

    const listener = commands.onJobUpdate.mock.calls[0][0] as (update: {
      jobId: string;
      state: string;
      step: string;
      percent?: number | null;
    }) => void;
    listener({
      jobId: "job-1",
      state: "running",
      step: "Reading text",
      percent: 42,
    });
    expect(onProgress).toHaveBeenCalledWith(42);
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("cancels the backend job when the signal aborts during OCR", async () => {
    let resolveOcr: ((value: AssistantText) => void) | undefined;
    commands.assistantOcrText.mockImplementation(
      () =>
        new Promise<AssistantText>((resolve) => {
          resolveOcr = resolve;
        }),
    );

    const runtime = createTauriIngestionRuntime();
    const controller = new AbortController();
    const promise = runtime.ocrText({
      path: "/tmp/report.pdf",
      pages: [1],
      language: "eng",
      signal: controller.signal,
    });

    await vi.waitFor(() =>
      expect(commands.assistantOcrText).toHaveBeenCalledTimes(1),
    );
    controller.abort();
    expect(commands.cancelJob).toHaveBeenCalledWith("job-1");

    resolveOcr?.({ pages: ["late text"], truncated: false });
    await expect(promise).rejects.toMatchObject({ name: "AbortError" });
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("does not start OCR when the signal is already aborted", async () => {
    const runtime = createTauriIngestionRuntime();
    const controller = new AbortController();
    controller.abort();

    await expect(
      runtime.ocrText({
        path: "/tmp/report.pdf",
        pages: [1],
        language: "eng",
        signal: controller.signal,
      }),
    ).rejects.toMatchObject({ name: "AbortError" });

    expect(commands.onJobUpdate).not.toHaveBeenCalled();
    expect(commands.assistantOcrText).not.toHaveBeenCalled();
    expect(commands.cancelJob).not.toHaveBeenCalled();
  });
});

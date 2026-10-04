// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { editsSignature, makeSourceTextObject, type SourceTextObject } from "@/lib/editor";
import type { AppError, TextEditIn, TextPreview } from "@/lib/types";

const backend = vi.hoisted(() => ({
  previewTextEdits: vi.fn<(path: string, fp: string, page: number, edits: TextEditIn[]) => Promise<unknown>>(),
}));
vi.mock("@/lib/tauriCommands", () => backend);
vi.mock("@/lib/pdfjs", () => ({ base64ToBytes: (b64: string) => new TextEncoder().encode(`pdf:${b64}`) }));

import { useTextPreview, type TextPreviewArgs, type TextPreviewController } from "./useTextPreview";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function change(runId: string, text: string): SourceTextObject {
  return makeSourceTextObject(`id-${runId}`, 0, { x: 0, y: 0, w: 10, h: 10 }, {
    runId,
    sourceFingerprint: "fp",
    sourcePageIndex: 0,
    originalText: "old",
    text,
    style: {},
  });
}

function preview(pagePdf: string | null, runId = "r1"): TextPreview {
  return {
    pagePdf,
    verdicts: [
      { runId, ok: true, code: null, chars: [], reason: null, face: null, field: null, detail: null, deltaPt: 0, newRect: null, caretOffsets: null },
    ],
    pageProblem: null,
    warnings: [],
  };
}

const decode = (bytes: Uint8Array | null) => (bytes ? new TextDecoder().decode(bytes) : null);

let root: Root | null = null;
let hook: TextPreviewController;
let render: (args: TextPreviewArgs) => void;

function mount(args: TextPreviewArgs) {
  function Probe(props: TextPreviewArgs) {
    hook = useTextPreview(props);
    return null;
  }
  const container = document.createElement("div");
  root = createRoot(container);
  render = (next) => act(() => root!.render(createElement(Probe, next)));
  render(args);
}

const base = { path: "/a.pdf", fingerprint: "fp", sourcePageIndex: 0 };

beforeEach(() => {
  backend.previewTextEdits.mockReset();
  // Calls a test does not answer stay pending.
  backend.previewTextEdits.mockImplementation(() => new Promise(() => {}));
});
afterEach(() => {
  act(() => root?.unmount());
  root = null;
});

describe("useTextPreview", () => {
  it("no changes ⇒ no bytes (the original render) and no call", () => {
    mount({ ...base, objects: [] });
    expect(hook.bytes).toBeNull();
    expect(hook.status).toBe("idle");
    expect(backend.previewTextEdits).not.toHaveBeenCalled();
  });

  it("previews the page's changes and decodes the page once ready", async () => {
    const d = deferred<unknown>();
    backend.previewTextEdits.mockReturnValueOnce(d.promise);
    mount({ ...base, objects: [change("r1", "new")] });
    expect(hook.status).toBe("pending");
    expect(backend.previewTextEdits).toHaveBeenCalledWith("/a.pdf", "fp", 0, [
      { runId: "r1", originalText: "old", text: "new", style: {} },
    ]);
    await act(async () => d.resolve(preview("AAA")));
    expect(hook.status).toBe("ready");
    expect(decode(hook.bytes)).toBe("pdf:AAA");
    expect(hook.result?.verdicts[0].ok).toBe(true);
  });

  it("pagePdf null ⇒ unavailable, original bytes", async () => {
    backend.previewTextEdits.mockResolvedValueOnce(preview(null));
    mount({ ...base, objects: [change("r1", "new")] });
    await act(async () => {});
    expect(hook.status).toBe("unavailable");
    expect(hook.bytes).toBeNull();
  });

  it("generation guard: a late answer for an older set of changes is dropped", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    backend.previewTextEdits.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    mount({ ...base, objects: [change("r1", "A")] });
    render({ ...base, objects: [change("r1", "B")] });
    await act(async () => second.resolve(preview("BBB")));
    await act(async () => first.resolve(preview("AAA")));
    expect(decode(hook.bytes)).toBe("pdf:BBB");
    expect(hook.status).toBe("ready");
  });

  it("caches by signature: going back to an earlier set does not call again", async () => {
    backend.previewTextEdits.mockResolvedValueOnce(preview("AAA")).mockResolvedValueOnce(preview("BBB"));
    mount({ ...base, objects: [change("r1", "A")] });
    await act(async () => {});
    render({ ...base, objects: [change("r1", "B")] });
    await act(async () => {});
    render({ ...base, objects: [change("r1", "A")] });
    await act(async () => {});
    expect(backend.previewTextEdits).toHaveBeenCalledTimes(2);
    expect(decode(hook.bytes)).toBe("pdf:AAA");
  });

  it("keeps showing the last bytes of the same page while a new set is checked, never another page's", async () => {
    const pending = deferred<unknown>();
    backend.previewTextEdits.mockResolvedValueOnce(preview("AAA")).mockReturnValueOnce(pending.promise);
    mount({ ...base, objects: [change("r1", "A")] });
    await act(async () => {});
    render({ ...base, objects: [change("r1", "B")] });
    expect(hook.status).toBe("pending");
    expect(decode(hook.bytes)).toBe("pdf:AAA");
    render({ ...base, sourcePageIndex: 1, objects: [change("r1", "B")] });
    expect(hook.bytes).toBeNull();
  });

  it("a source page listed twice never shows the other listing's bytes while pending", async () => {
    const pending = deferred<unknown>();
    backend.previewTextEdits.mockResolvedValueOnce(preview("PAGE0")).mockReturnValueOnce(pending.promise);
    mount({ ...base, objects: [change("r1", "A")] });
    await act(async () => {});
    expect(decode(hook.bytes)).toBe("pdf:PAGE0");
    const onOtherListing = { ...change("r1", "B"), pageIndex: 5 };
    render({ ...base, objects: [onOtherListing] });
    expect(hook.status).toBe("pending");
    expect(hook.bytes).toBeNull();
  });

  it("seed: the commit's own preview is shown without a second call", async () => {
    mount({ ...base, objects: [] });
    const next = [change("r1", "new")];
    act(() => hook.seed(editsSignature(next), preview("SEED")));
    render({ ...base, objects: next });
    await act(async () => {});
    expect(backend.previewTextEdits).not.toHaveBeenCalled();
    expect(decode(hook.bytes)).toBe("pdf:SEED");
  });

  it("request() checks a candidate set without changing the shown page; errors reject as AppError", async () => {
    backend.previewTextEdits.mockResolvedValueOnce(preview("CAND"));
    mount({ ...base, objects: [] });
    const answer = await hook.request([{ runId: "r1", originalText: "old", text: "x", style: {} }]);
    expect(answer.pagePdf).toBe("CAND");
    expect(hook.bytes).toBeNull();
    const stale: AppError = { code: "STALE", title: "The PDF changed on disk", message: "m" };
    backend.previewTextEdits.mockRejectedValueOnce(stale);
    await expect(hook.request([])).rejects.toEqual(stale);
  });

  it("a backend error is kept (never swallowed) and shows the original", async () => {
    const err: AppError = { code: "VERIFIER_MISSING", title: "A checking component is missing", message: "m" };
    backend.previewTextEdits.mockRejectedValueOnce(err);
    mount({ ...base, objects: [change("r1", "new")] });
    await act(async () => {});
    expect(hook.status).toBe("error");
    expect(hook.error).toEqual(err);
    expect(hook.bytes).toBeNull();
  });
});

import { describe, expect, it, vi } from "vitest";
import { makeRectObject, makeRedactObject, makeSourceTextObject, type EditObject } from "@/lib/editor";
import type { PageRef } from "@/lib/types";
import { finishOpenTextEdit, textSaveGuard } from "./textSaveGuards";
import type { TextSourceState } from "./useTextSources";

const A = "/docs/a.pdf";
const B = "/docs/b.pdf";

function ref(path: string, page: number, uid = path): PageRef {
  return { key: `${uid}#${page}`, path, page, fileName: path.split("/").pop() ?? path };
}

const REFS: PageRef[] = [ref(A, 1), ref(A, 2), ref(B, 1)];

function ready(fingerprint = "fp-a", stale = false): TextSourceState {
  return { status: "ready", info: { fingerprint, pageCount: 2, warnings: [] }, error: null, stale };
}

const SOURCES: Record<string, TextSourceState> = { [A]: ready("fp-a"), [B]: ready("fp-b") };

function change(pageIndex: number, sourcePageIndex: number, fingerprint = "fp-a"): EditObject {
  return makeSourceTextObject(`t${pageIndex}`, pageIndex, { x: 72, y: 697, w: 40, h: 12 }, {
    runId: `t1:${fingerprint}:${sourcePageIndex}:1-2`,
    sourceFingerprint: fingerprint,
    sourcePageIndex,
    originalText: "Hello",
    text: "Hallo",
    style: {},
  });
}

describe("textSaveGuard", () => {
  it("passes with no text changes, whatever the sources say", () => {
    expect(textSaveGuard({ objects: [makeRectObject("r", 0, { x: 0, y: 0, w: 5, h: 5 })], refs: REFS, sources: {} })).toBeNull();
  });

  it("passes when every change has a ready, current source", () => {
    const objects = [change(1, 1), change(2, 0, "fp-b"), makeRedactObject("x", 0, { x: 0, y: 0, w: 5, h: 5 })];
    expect(textSaveGuard({ objects, refs: REFS, sources: SOURCES })).toBeNull();
  });

  it("asks to reopen Edit text when the source is not ready", () => {
    const expected = { title: "Text changes can't be saved yet", description: "Open Edit text on “a.pdf” again, then save." };
    const loading: TextSourceState = { status: "loading", info: null, error: null, stale: false };
    const failed: TextSourceState = { status: "error", info: null, error: { code: "X", title: "t", message: "m" }, stale: false };
    for (const state of [undefined, loading, failed]) {
      const sources: Record<string, TextSourceState> = state ? { ...SOURCES, [A]: state } : { [B]: SOURCES[B] };
      expect(textSaveGuard({ objects: [change(0, 0)], refs: REFS, sources })).toEqual(expected);
    }
  });

  it("refuses changes made on an older snapshot of the file", () => {
    const expected = {
      title: "The PDF changed on disk",
      description: "“a.pdf” was changed after you started editing it, so your text changes no longer match it.",
    };
    expect(textSaveGuard({ objects: [change(0, 0)], refs: REFS, sources: { ...SOURCES, [A]: ready("fp-a", true) } })).toEqual(
      expected,
    );
    expect(textSaveGuard({ objects: [change(0, 0, "fp-old")], refs: REFS, sources: SOURCES })).toEqual(expected);
    expect(textSaveGuard({ objects: [change(0, 1)], refs: REFS, sources: SOURCES })).toEqual(expected);
  });

  it("refuses a redaction and a text change on the same page", () => {
    const objects = [change(1, 1), makeRedactObject("x", 1, { x: 0, y: 0, w: 5, h: 5 })];
    expect(textSaveGuard({ objects, refs: REFS, sources: SOURCES })).toEqual({
      title: "Redaction and text change on the same page",
      description:
        "Page 2 has both a redaction and a text change. Redaction turns the page into an image, so the text change would be lost. Remove the redaction or the text change on that page.",
    });
  });

  it("refuses a change on a page listed twice", () => {
    const refs = [ref(A, 1), ref(A, 2), ref(A, 1, "copy")];
    expect(textSaveGuard({ objects: [change(0, 0)], refs, sources: SOURCES })).toEqual({
      title: "This page appears twice",
      description:
        "Page 1 of “a.pdf” is in the list more than once and has a text change. Remove the extra copy of the page, then save again.",
    });
    expect(textSaveGuard({ objects: [change(1, 1)], refs, sources: SOURCES })).toBeNull();
  });

  it("reports the first failing check in order: not ready, stale, redaction, duplicate", () => {
    const refs = [ref(A, 1), ref(A, 1, "copy"), ref(B, 1)];
    const redact = makeRedactObject("x", 0, { x: 0, y: 0, w: 5, h: 5 });
    const objects = [change(0, 0), redact, change(2, 0, "fp-b")];
    const notReadyB = { ...SOURCES, [B]: { status: "loading", info: null, error: null, stale: false } as TextSourceState };
    expect(textSaveGuard({ objects, refs, sources: notReadyB })?.title).toBe("Text changes can't be saved yet");
    expect(textSaveGuard({ objects, refs, sources: { ...SOURCES, [B]: ready("fp-b", true) } })?.title).toBe(
      "The PDF changed on disk",
    );
    expect(textSaveGuard({ objects, refs, sources: SOURCES })?.title).toBe("Redaction and text change on the same page");
    expect(textSaveGuard({ objects: [change(0, 0)], refs, sources: SOURCES })?.title).toBe("This page appears twice");
  });
});

describe("finishOpenTextEdit (Save with a line edit still open)", () => {
  const guard = (editing: boolean, result: boolean | Promise<boolean>) => {
    const tryClose = vi.fn(() => Promise.resolve(result));
    return { edit: { isEditing: () => editing, tryClose }, tryClose };
  };

  it("does nothing when no line is open", async () => {
    const { edit, tryClose } = guard(false, true);
    expect(await finishOpenTextEdit(edit)).toBe("none");
    expect(await finishOpenTextEdit(null)).toBe("none");
    expect(tryClose).not.toHaveBeenCalled();
  });

  it("waits for the open edit's check and reports it finished, so Save reads the objects again", async () => {
    let resolve: (ok: boolean) => void = () => {};
    const pending = new Promise<boolean>((r) => (resolve = r));
    const { edit, tryClose } = guard(true, pending);
    const out = finishOpenTextEdit(edit);
    let settled = false;
    void out.then(() => (settled = true));
    await Promise.resolve();
    expect(settled).toBe(false);
    resolve(true);
    expect(await out).toBe("finished");
    expect(tryClose).toHaveBeenCalledTimes(1);
  });

  it("reports a draft that can't be applied as blocked", async () => {
    const { edit } = guard(true, false);
    expect(await finishOpenTextEdit(edit)).toBe("blocked");
  });
});

import { describe, it, expect } from "vitest";
import {
  createHistoryState,
  editReducer,
  canUndo,
  canRedo,
  makeRectObject,
  makeSourceTextObject,
  setSourceTextAction,
  MAX_HISTORY,
  type HistoryState,
} from "./editReducer";
import { createEmptyDocument, type EditObject, type SourceTextObject } from "./types";
import type { SourceTextStyle, TextRect, TextRun } from "../types";

function rect(id: string, pageIndex = 0) {
  return makeRectObject(id, pageIndex, { x: 10, y: 20, w: 100, h: 50 });
}

describe("editReducer", () => {
  it("starts empty", () => {
    const s = createHistoryState();
    expect(s.present).toEqual(createEmptyDocument());
    expect(canUndo(s)).toBe(false);
    expect(canRedo(s)).toBe(false);
  });

  it("adds an object and selects it", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    expect(s.present.objects).toHaveLength(1);
    expect(s.present.selectedIds).toEqual(["a"]);
    expect(canUndo(s)).toBe(true);
  });

  it("undo / redo add", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects).toHaveLength(0);
    expect(canRedo(s)).toBe(true);
    s = editReducer(s, { type: "REDO" });
    expect(s.present.objects[0].id).toBe("a");
  });

  it("delete multi-select", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    s = editReducer(s, { type: "ADD", object: rect("b") });
    s = editReducer(s, { type: "ADD", object: rect("c") });
    s = editReducer(s, { type: "DELETE", ids: ["a", "c"] });
    expect(s.present.objects.map((o) => o.id)).toEqual(["b"]);
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects).toHaveLength(3);
  });

  it("select does not create history entries", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    s = editReducer(s, { type: "ADD", object: rect("b") });
    const pastLen = s.past.length;
    s = editReducer(s, { type: "SELECT", ids: ["a"] });
    s = editReducer(s, { type: "SELECT", ids: ["b"] });
    s = editReducer(s, { type: "CLEAR_SELECTION" });
    expect(s.past.length).toBe(pastLen);
    expect(s.present.selectedIds).toEqual([]);
  });

  it("gesture coalesce: one undo restores pre-drag rect", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    const start = s.present.objects[0].rect;

    s = editReducer(s, { type: "BEGIN_GESTURE" });
    s = editReducer(s, {
      type: "UPDATE",
      id: "a",
      patch: { rect: { x: 50, y: 60, w: 100, h: 50 } },
    });
    s = editReducer(s, {
      type: "UPDATE",
      id: "a",
      patch: { rect: { x: 80, y: 90, w: 100, h: 50 } },
    });
    s = editReducer(s, { type: "END_GESTURE" });

    expect(s.present.objects[0].rect).toEqual({ x: 80, y: 90, w: 100, h: 50 });
    // Only one history step for the whole gesture beyond ADD
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects[0].rect).toEqual(start);
    // Another undo removes the object
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects).toHaveLength(0);
  });

  it("UPDATE without gesture still creates history", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    s = editReducer(s, {
      type: "UPDATE",
      id: "a",
      patch: { rect: { x: 1, y: 2, w: 3, h: 4 } },
    });
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects[0].rect).toEqual({ x: 10, y: 20, w: 100, h: 50 });
  });

  it("caps history length", () => {
    let s = createHistoryState();
    for (let i = 0; i < MAX_HISTORY + 20; i++) {
      s = editReducer(s, { type: "ADD", object: rect(`id-${i}`) });
    }
    expect(s.past.length).toBeLessThanOrEqual(MAX_HISTORY);
  });

  it("REBIND keeps provided history snapshots", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    s = editReducer(s, {
      type: "REBIND",
      present: { version: 1, objects: [rect("a", 0)], selectedIds: ["a"] },
      past: [{ version: 1, objects: [], selectedIds: [] }],
      future: [],
    });
    expect(s.present.objects[0].id).toBe("a");
    expect(canUndo(s)).toBe(true);
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects).toHaveLength(0);
  });

  it("REPLACE resets history", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    s = editReducer(s, {
      type: "REPLACE",
      document: {
        version: 1,
        objects: [rect("z")],
        selectedIds: [],
      },
    });
    expect(s.present.objects[0].id).toBe("z");
    expect(canUndo(s)).toBe(false);
  });

  it("ADD_MANY inserts all and selects them as one undo step", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    s = editReducer(s, {
      type: "ADD_MANY",
      objects: [rect("b"), rect("c")],
    });
    expect(s.present.objects.map((o) => o.id)).toEqual(["a", "b", "c"]);
    expect(s.present.selectedIds).toEqual(["b", "c"]);
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects.map((o) => o.id)).toEqual(["a"]);
  });

  it("reorders layers on the same page only", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a", 0) });
    s = editReducer(s, { type: "ADD", object: rect("b", 0) });
    s = editReducer(s, { type: "ADD", object: rect("c", 1) });
    s = editReducer(s, { type: "REORDER", id: "a", dir: "front" });
    // Page 0 only: A moves in front of B; C stays on page 1.
    expect(s.present.objects.map((o) => o.id)).toEqual(["b", "a", "c"]);
    s = editReducer(s, { type: "REORDER", id: "a", dir: "backward" });
    expect(s.present.objects.map((o) => o.id)).toEqual(["a", "b", "c"]);
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects.map((o) => o.id)).toEqual(["b", "a", "c"]);
  });

  it("no-op gesture does not clear redo", () => {
    let s = createHistoryState();
    s = editReducer(s, { type: "ADD", object: rect("a") });
    s = editReducer(s, { type: "ADD", object: rect("b") });
    s = editReducer(s, { type: "UNDO" });
    expect(s.present.objects.map((o) => o.id)).toEqual(["a"]);
    expect(canRedo(s)).toBe(true);

    s = editReducer(s, { type: "BEGIN_GESTURE" });
    s = editReducer(s, { type: "END_GESTURE" });
    expect(canRedo(s)).toBe(true);
    expect(s.present.objects.map((o) => o.id)).toEqual(["a"]);
  });
});

// ---------------------------------------------------------------------------
// sourceText (Edit text, v0.4)
// ---------------------------------------------------------------------------

function textRun(id = "t1:fp:0:10-20", text = "Hello"): TextRun {
  return {
    id,
    order: 0,
    line: 0,
    text,
    rect: { x: 72, y: 697, w: 27, h: 12 },
    origin: { x: 72, y: 700 },
    dir: { x: 1, y: 0 },
    ascent: 9,
    descent: 3,
    caretOffsets: [],
    editable: true,
    reason: null,
    metrics: {
      surface: ["f1"],
      tfSize: 12,
      effectiveSize: 12,
      charSpacing: 0,
      wordSpacing: 0,
      hScale: 1,
      textToUser: 1,
      letterSpacingPt: 0,
      spaceMode: "glyph",
      kernSpace: -250,
      originalWidth: 27,
      visibleExtent: 500,
      nextObstacle: null,
    },
    style: {
      fill: "#000000",
      sizeChangeable: true,
      colourChangeable: true,
      face: "regular",
      faces: {
        regular: { available: true, surface: ["f1"] },
        bold: { available: false, surface: [] },
        italic: { available: false, surface: [] },
        boldItalic: { available: false, surface: [] },
      },
    },
    substituted: false,
  };
}

function setText(
  s: HistoryState,
  text: string,
  style: SourceTextStyle = {},
  opts: { pageIndex?: number; run?: TextRun; id?: string; fingerprint?: string; rect?: TextRect } = {},
): HistoryState {
  const input = {
    pageIndex: opts.pageIndex ?? 0,
    run: opts.run ?? textRun(),
    sourceFingerprint: opts.fingerprint ?? "fp",
    sourcePageIndex: 0,
    text,
    style,
    rect: opts.rect,
  };
  return editReducer(s, setSourceTextAction(input, opts.id ?? `id-${s.past.length}`));
}

function sourceTexts(s: HistoryState): SourceTextObject[] {
  return s.present.objects.filter((o): o is SourceTextObject => o.kind === "sourceText");
}

describe("editReducer sourceText", () => {
  it("builds a locked object bound to its run", () => {
    const o = makeSourceTextObject("o", 2, { x: 1, y: 2, w: 3, h: 4 }, {
      runId: "r",
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: "a",
      text: "b",
      style: { sizePt: 13 },
    });
    expect(o).toEqual({
      id: "o",
      kind: "sourceText",
      pageIndex: 2,
      rect: { x: 1, y: 2, w: 3, h: 4 },
      locked: true,
      runId: "r",
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: "a",
      text: "b",
      style: { sizePt: 13 },
    });
  });

  it("stores NFC text, a normalised style and the verdict's new rect", () => {
    const decomposed = `Ye${String.fromCharCode(0x67, 0x306)}`;
    const s = setText(createHistoryState(), decomposed, { sizePt: 12, fill: "#FF0000" }, { rect: { x: 72, y: 697, w: 40, h: 12 } });
    const [o] = sourceTexts(s);
    expect(o.text).toBe("Yeğ");
    expect(o.style).toEqual({ fill: "#ff0000" });
    expect(o.rect).toEqual({ x: 72, y: 697, w: 40, h: 12 });
    expect(o.originalText).toBe("Hello");
    expect(canUndo(s)).toBe(true);
  });

  it("upserts per (pageIndex, runId) with one history step each", () => {
    let s = setText(createHistoryState(), "Hallo", {}, { id: "first" });
    s = setText(s, "Hullo", { sizePt: 14 }, { id: "second", rect: { x: 72, y: 697, w: 30, h: 14 } });
    expect(sourceTexts(s)).toHaveLength(1);
    expect(sourceTexts(s)[0]).toMatchObject({ id: "first", text: "Hullo", style: { sizePt: 14 }, rect: { w: 30, h: 14 } });
    expect(s.past).toHaveLength(2);
    s = setText(s, "Hello", {}, { pageIndex: 1, id: "other-page" });
    expect(sourceTexts(s)).toHaveLength(1); // a no-op on page 1 adds nothing
    s = setText(s, "Hey", {}, { pageIndex: 1, id: "other-page" });
    expect(sourceTexts(s).map((o) => [o.id, o.pageIndex])).toEqual([
      ["first", 0],
      ["other-page", 1],
    ]);
  });

  it("does not add a history step when nothing changes", () => {
    let s = setText(createHistoryState(), "Hallo");
    const before = s;
    s = setText(s, "Hallo");
    expect(s).toBe(before);
  });

  it("deletes the change when the line is set back to the original (B7)", () => {
    let s = setText(createHistoryState(), "Hallo", { fill: "#ff0000" });
    s = setText(s, "Hello", { fill: "#000000", sizePt: 12 });
    expect(sourceTexts(s)).toHaveLength(0);
    expect(s.past).toHaveLength(2);
    const unchanged = setText(createHistoryState(), "Hello");
    expect(unchanged.past).toHaveLength(0);
  });

  it("undo and redo restore text and style", () => {
    let s = setText(createHistoryState(), "Hallo", { sizePt: 14 });
    s = setText(s, "Hullo", { fill: "#c71c1c" });
    s = editReducer(s, { type: "UNDO" });
    expect(sourceTexts(s)[0]).toMatchObject({ text: "Hallo", style: { sizePt: 14 } });
    s = editReducer(s, { type: "REDO" });
    expect(sourceTexts(s)[0]).toMatchObject({ text: "Hullo", style: { fill: "#c71c1c" } });
    s = editReducer(s, { type: "UNDO" });
    s = editReducer(s, { type: "UNDO" });
    expect(sourceTexts(s)).toHaveLength(0);
  });

  it("rejects rect, rotation, opacity, lock, identity and binding patches", () => {
    let s = setText(createHistoryState(), "Hallo", {}, { id: "t" });
    const original = sourceTexts(s)[0];
    s = editReducer(s, {
      type: "UPDATE",
      id: "t",
      patch: {
        rect: { x: 0, y: 0, w: 1, h: 1 },
        objectRotate: 45,
        opacity: 0.5,
        locked: false,
        id: "hijack",
        runId: "other",
        sourceFingerprint: "x",
        sourcePageIndex: 9,
        pageIndex: 3,
        kind: "rect",
      } as Partial<EditObject>,
    });
    expect(sourceTexts(s)[0]).toEqual(original);
    s = editReducer(s, { type: "UPDATE", id: "t", patch: { text: "Hxllo", style: { sizePt: 20 } } as Partial<EditObject> });
    expect(sourceTexts(s)[0]).toEqual({ ...original, text: "Hxllo", style: { sizePt: 20 } });
  });

  it("removes every change of one snapshot in one step", () => {
    let s = setText(createHistoryState(), "A1", {}, { id: "a" });
    s = setText(s, "B1", {}, { id: "b", run: textRun("t1:fp:0:30-40", "World") });
    s = setText(s, "C1", {}, { id: "c", fingerprint: "other", run: textRun("t1:other:0:1-2") });
    s = editReducer(s, { type: "ADD", object: rect("shape") });
    const steps = s.past.length;
    s = editReducer(s, { type: "REMOVE_SOURCE_TEXT_FOR_FINGERPRINT", fingerprint: "fp" });
    expect(s.present.objects.map((o) => o.id)).toEqual(["c", "shape"]);
    expect(s.past.length).toBe(steps + 1);
    const same = editReducer(s, { type: "REMOVE_SOURCE_TEXT_FOR_FINGERPRINT", fingerprint: "missing" });
    expect(same).toBe(s);
  });

  it("never pastes or duplicates a text change", () => {
    let s = setText(createHistoryState(), "Hallo", {}, { id: "t" });
    const copy = { ...sourceTexts(s)[0], id: "t2" };
    const before = s;
    s = editReducer(s, { type: "ADD_MANY", objects: [copy] });
    expect(s).toBe(before);
    s = editReducer(s, { type: "ADD_MANY", objects: [copy, rect("r")] });
    expect(s.present.objects.map((o) => o.id)).toEqual(["t", "r"]);
  });
});

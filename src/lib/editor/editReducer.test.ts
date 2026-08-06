import { describe, it, expect } from "vitest";
import {
  createHistoryState,
  editReducer,
  canUndo,
  canRedo,
  makeRectObject,
  MAX_HISTORY,
} from "./editReducer";
import { createEmptyDocument } from "./types";

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
});

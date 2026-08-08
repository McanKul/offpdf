import { describe, it, expect } from "vitest";
import { makeRectObject } from "./editReducer";
import { createEmptyDocument } from "./types";
import { planKeyRebind, remapEditDocument, resolveViewPageIndex } from "./remapPages";

function doc(objects: ReturnType<typeof makeRectObject>[], selectedIds: string[] = []) {
  return { version: 1 as const, objects, selectedIds };
}

function rect(id: string, pageIndex: number) {
  return makeRectObject(id, pageIndex, { x: 10, y: 20, w: 100, h: 50 });
}

describe("remapEditDocument", () => {
  it("keeps pageIndex when a file is added at the end", () => {
    const oldKeys = ["f1#1", "f1#2"];
    const newKeys = ["f1#1", "f1#2", "f2#1"];
    const r = remapEditDocument(doc([rect("a", 1)], ["a"]), oldKeys, newKeys);
    expect(r.droppedIds).toEqual([]);
    expect(r.document.objects[0].pageIndex).toBe(1);
    expect(r.document.selectedIds).toEqual(["a"]);
  });

  it("decrements pageIndex when an unedited file is removed before the edit", () => {
    const oldKeys = ["f1#1", "f2#1", "f2#2"];
    const newKeys = ["f2#1", "f2#2"];
    const r = remapEditDocument(doc([rect("a", 2)]), oldKeys, newKeys);
    expect(r.droppedIds).toEqual([]);
    expect(r.document.objects[0].id).toBe("a");
    expect(r.document.objects[0].pageIndex).toBe(1);
  });

  it("drops objects whose page key disappeared", () => {
    const oldKeys = ["f1#1", "f2#1"];
    const newKeys = ["f1#1"];
    const r = remapEditDocument(doc([rect("keep", 0), rect("gone", 1)], ["keep", "gone"]), oldKeys, newKeys);
    expect(r.droppedIds).toEqual(["gone"]);
    expect(r.document.objects.map((o) => o.id)).toEqual(["keep"]);
    expect(r.document.selectedIds).toEqual(["keep"]);
  });

  it("does not alias edits when the same file is added twice with different uids", () => {
    const oldKeys = ["f1#1"];
    const newKeys = ["f1#1", "f2#1"];
    const r = remapEditDocument(doc([rect("a", 0)]), oldKeys, newKeys);
    expect(r.droppedIds).toEqual([]);
    expect(r.document.objects[0].pageIndex).toBe(0);
    expect(r.document.objects).toHaveLength(1);
  });

  it("drops objects with an out-of-range pageIndex", () => {
    const r = remapEditDocument(doc([rect("a", 3)]), ["f1#1"], ["f1#1"]);
    expect(r.droppedIds).toEqual(["a"]);
    expect(r.document.objects).toEqual([]);
  });
});

describe("planKeyRebind", () => {
  it("is a no-op when keys are unchanged", () => {
    const present = doc([rect("a", 0)]);
    expect(planKeyRebind(present, [], [], ["f1#1"], ["f1#1"])).toBeNull();
  });

  it("keeps remapped undo history when the remap is lossless", () => {
    const present = doc([rect("a", 1)]);
    const past = [doc([rect("a", 1)]), createEmptyDocument()];
    const plan = planKeyRebind(present, past, [], ["f1#1", "f2#1"], ["f2#1"]);
    expect(plan).not.toBeNull();
    expect(plan!.droppedIds).toEqual([]);
    expect(plan!.historyDropped).toBe(false);
    expect(plan!.present.objects[0].pageIndex).toBe(0);
    expect(plan!.past).toHaveLength(2);
    expect(plan!.past[0].objects[0].pageIndex).toBe(0);
    expect(plan!.past[1].objects).toHaveLength(0);
  });

  it("clears history when present objects would be dropped", () => {
    const present = doc([rect("a", 0)]);
    const past = [createEmptyDocument()];
    const plan = planKeyRebind(present, past, [], ["f1#1", "f2#1"], ["f2#1"]);
    expect(plan!.droppedIds).toEqual(["a"]);
    expect(plan!.historyDropped).toBe(false);
    expect(plan!.present.objects).toHaveLength(0);
    expect(plan!.past).toEqual([]);
    expect(plan!.future).toEqual([]);
  });

  it("clears history when only a past snapshot would drop objects", () => {
    const present = createEmptyDocument();
    const past = [doc([rect("a", 1)])];
    const plan = planKeyRebind(present, past, [], ["f1#1", "f2#1"], ["f1#1"]);
    expect(plan!.droppedIds).toEqual([]);
    expect(plan!.historyDropped).toBe(true);
    expect(plan!.past).toEqual([]);
    expect(plan!.future).toEqual([]);
  });
});

describe("resolveViewPageIndex", () => {
  it("follows the previous page key after an earlier file is removed", () => {
    const next = ["f2#1", "f2#2"];
    expect(resolveViewPageIndex(next, 3, "f2#1")).toBe(0);
    expect(resolveViewPageIndex(next, 4, "f2#2")).toBe(1);
  });

  it("clamps when the viewed page key disappeared", () => {
    expect(resolveViewPageIndex(["f2#1"], 3, "f1#2")).toBe(0);
  });

  it("keeps the index when the key is still at that slot", () => {
    expect(resolveViewPageIndex(["f1#1", "f2#1"], 0, "f1#1")).toBe(0);
  });

  it("still follows prevKey even if currentIndex already points at another live page", () => {
    // After removing an earlier file, pageIndex may still be in range but now
    // names a different page. Follow the previous key, not the stale index.
    expect(resolveViewPageIndex(["f2#1", "f2#2"], 0, "f2#2")).toBe(1);
  });
});

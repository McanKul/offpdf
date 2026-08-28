import { describe, it, expect } from "vitest";
import {
  emptyObjectsBlockSave,
  incompleteSourceIds,
  incompleteSourcePaths,
  shouldRewriteSourceLinks,
} from "./linkSavePolicy";

describe("emptyObjectsBlockSave", () => {
  it("H6-ui: empty objects + hadHydratedLinks does not block Save", () => {
    expect(
      emptyObjectsBlockSave({ objectCount: 0, hadHydratedLinks: true }),
    ).toBe(false);
  });

  it("H6-ui: empty objects + never hydrated still blocks", () => {
    expect(
      emptyObjectsBlockSave({ objectCount: 0, hadHydratedLinks: false }),
    ).toBe(true);
  });
});

describe("shouldRewriteSourceLinks", () => {
  it("H7-ui: per-failed-source paths; mixed-source edit is allowed", () => {
    const files = [
      { uid: "A", path: "/a.pdf" },
      { uid: "B", path: "/b.pdf" },
    ];
    const failed = new Set(["A"]);
    const incomplete = incompleteSourceIds(
      files.map((f) => f.uid),
      failed,
    );
    expect(incomplete).toEqual(["A"]);
    expect(incompleteSourcePaths(files, failed)).toEqual(["/a.pdf"]);
    expect(
      shouldRewriteSourceLinks({
        sourceId: "B",
        incompleteSourceIds: incomplete,
      }),
    ).toBe(true);
    expect(
      shouldRewriteSourceLinks({
        sourceId: "A",
        incompleteSourceIds: incomplete,
      }),
    ).toBe(false);
  });
});

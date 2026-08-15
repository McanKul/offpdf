import { describe, it, expect } from "vitest";
import { clampPageIndex } from "./pageIndex";

describe("clampPageIndex", () => {
  it("S1: clamps stale and out-of-range indices into [0, pageCount-1]", () => {
    expect(clampPageIndex(2, 1)).toBe(0);
    expect(clampPageIndex(0, 3)).toBe(0);
    expect(clampPageIndex(1, 3)).toBe(1);
    expect(clampPageIndex(5, 0)).toBe(0);
    expect(clampPageIndex(-1, 4)).toBe(0);
  });

  it("S2: after a 3-page list shrinks to 1 page, clamp alone maps stale index 2 to 0", () => {
    // Was on page index 2 of 3. The list shrinks to 1 page and the samePageKeys
    // branch is skipped — clamp must still run every render.
    const staleIndex = 2;
    const wasPageCount = 3;
    expect(clampPageIndex(staleIndex, wasPageCount)).toBe(2);

    const newPageCount = 1;
    expect(clampPageIndex(staleIndex, newPageCount)).toBe(0);
  });
});

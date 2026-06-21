import { describe, it, expect } from "vitest";
import {
  parsePageRange,
  formatPageList,
  invertPages,
} from "./pageRange";

describe("parsePageRange (set mode)", () => {
  it("parses a single page", () => {
    expect(parsePageRange("1")).toEqual({ ok: true, pages: [1] });
  });

  it("parses a comma list", () => {
    expect(parsePageRange("1,2,3")).toEqual({ ok: true, pages: [1, 2, 3] });
  });

  it("parses a range", () => {
    expect(parsePageRange("1-5")).toEqual({ ok: true, pages: [1, 2, 3, 4, 5] });
  });

  it("parses a mixed list", () => {
    expect(parsePageRange("1,3,5-8")).toEqual({
      ok: true,
      pages: [1, 3, 5, 6, 7, 8],
    });
  });

  it("ignores all whitespace", () => {
    expect(parsePageRange("  1 , 3 , 5 - 8 ")).toEqual({
      ok: true,
      pages: [1, 3, 5, 6, 7, 8],
    });
  });

  it("dedupes and sorts in set mode", () => {
    expect(parsePageRange("3,1,3,2")).toEqual({ ok: true, pages: [1, 2, 3] });
  });

  it("expands 'all' when page count is known", () => {
    expect(parsePageRange("all", { pageCount: 3 })).toEqual({
      ok: true,
      pages: [1, 2, 3],
    });
  });

  it("rejects 'all' without page count", () => {
    const r = parsePageRange("all");
    expect(r.ok).toBe(false);
  });

  it("rejects page 0", () => {
    expect(parsePageRange("0").ok).toBe(false);
  });

  it("rejects negative pages", () => {
    expect(parsePageRange("-3").ok).toBe(false);
  });

  it("rejects non-numeric input", () => {
    expect(parsePageRange("a,b").ok).toBe(false);
  });

  it("rejects empty input", () => {
    expect(parsePageRange("").ok).toBe(false);
  });

  it("rejects pages beyond the document length", () => {
    const r = parsePageRange("1,99", { pageCount: 10 });
    expect(r.ok).toBe(false);
  });

  it("rejects malformed ranges", () => {
    expect(parsePageRange("1-").ok).toBe(false);
    expect(parsePageRange("-5").ok).toBe(false);
    expect(parsePageRange("1,,2").ok).toBe(false);
  });
});

describe("parsePageRange (reorder/preserveOrder mode)", () => {
  it("preserves the exact order", () => {
    expect(
      parsePageRange("1,3,2,4-10", { preserveOrder: true, pageCount: 10 }),
    ).toEqual({ ok: true, pages: [1, 3, 2, 4, 5, 6, 7, 8, 9, 10] });
  });

  it("keeps duplicates in reorder mode", () => {
    expect(parsePageRange("1,1,2", { preserveOrder: true })).toEqual({
      ok: true,
      pages: [1, 1, 2],
    });
  });

  it("expands descending ranges in reverse", () => {
    expect(parsePageRange("5-3", { preserveOrder: true })).toEqual({
      ok: true,
      pages: [5, 4, 3],
    });
  });
});

describe("formatPageList", () => {
  it("compacts consecutive runs into ranges", () => {
    expect(formatPageList([1, 2, 3, 5, 7, 8])).toBe("1-3,5,7-8");
  });
  it("handles a single page", () => {
    expect(formatPageList([4])).toBe("4");
  });
  it("handles empty", () => {
    expect(formatPageList([])).toBe("");
  });
});

describe("invertPages", () => {
  it("returns the complement to keep when deleting", () => {
    expect(invertPages([2, 5], 6)).toEqual([1, 3, 4, 6]);
  });
});

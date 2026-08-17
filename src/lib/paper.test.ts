import { describe, expect, it } from "vitest";
import { describeSize, gridFor, paperPt, PT_PER_MM, tileCount } from "./paper";

describe("paperPt", () => {
  it("converts A4 millimetres to points in portrait orientation", () => {
    const size = paperPt("a4", true);

    expect(size.w).toBeCloseTo(210 * PT_PER_MM);
    expect(size.h).toBeCloseTo(297 * PT_PER_MM);
  });

  it("swaps the paper dimensions in landscape orientation", () => {
    const portrait = paperPt("a4", true);
    const landscape = paperPt("a4", false);

    expect(landscape.w).toBeCloseTo(portrait.h);
    expect(landscape.h).toBeCloseTo(portrait.w);
  });
});

describe("tileCount", () => {
  it("uses one tile for an exact fit", () => {
    expect(tileCount(100, 100, 0)).toBe(1);
  });

  it("allows the 0.5 point fit tolerance", () => {
    expect(tileCount(100.5, 100, 0)).toBe(1);
  });

  it("adds a tile at the first overflow beyond the tolerance", () => {
    expect(tileCount(100.500001, 100, 0)).toBe(2);
  });

  it("counts multiple sheets along one dimension", () => {
    expect(tileCount(300, 100, 0)).toBe(3);
  });

  it("increases the sheet count when overlap reduces the usable step", () => {
    expect(tileCount(300, 100, 25)).toBe(4);
  });
});

describe("gridFor", () => {
  it("honours explicit portrait and landscape orientations", () => {
    const portrait = gridFor(1_000, 1_000, "a4", "portrait", 0);
    const landscape = gridFor(1_000, 1_000, "a4", "landscape", 0);

    expect(portrait.tileW).toBeCloseTo(210 * PT_PER_MM);
    expect(portrait.tileH).toBeCloseTo(297 * PT_PER_MM);
    expect(landscape.tileW).toBeCloseTo(297 * PT_PER_MM);
    expect(landscape.tileH).toBeCloseTo(210 * PT_PER_MM);
  });

  it("reports count as the product of columns and rows", () => {
    const grid = gridFor(1_600, 1_600, "a4", "portrait", 0);

    expect(grid.count).toBe(grid.cols * grid.rows);
  });

  it("selects portrait paper for a tall page when it uses fewer sheets", () => {
    const grid = gridFor(500, 1_600, "a4", "auto", 0);

    expect(grid.count).toBe(2);
    expect(grid.tileW).toBeCloseTo(210 * PT_PER_MM);
    expect(grid.tileH).toBeCloseTo(297 * PT_PER_MM);
  });

  it("selects landscape paper for a wide page when it uses fewer sheets", () => {
    const grid = gridFor(1_600, 500, "a4", "auto", 0);

    expect(grid.count).toBe(2);
    expect(grid.tileW).toBeCloseTo(297 * PT_PER_MM);
    expect(grid.tileH).toBeCloseTo(210 * PT_PER_MM);
  });

  it("selects portrait deterministically when orientations tie", () => {
    const grid = gridFor(500, 500, "a4", "auto", 0);

    expect(grid.count).toBe(1);
    expect(grid.tileW).toBeCloseTo(210 * PT_PER_MM);
    expect(grid.tileH).toBeCloseTo(297 * PT_PER_MM);
  });
});

describe("describeSize", () => {
  it("recognises A-series sizes in either orientation", () => {
    expect(describeSize(210 * PT_PER_MM, 297 * PT_PER_MM)).toBe(
      "A4 (210 × 297 mm)",
    );
    expect(describeSize(420 * PT_PER_MM, 297 * PT_PER_MM)).toBe(
      "A3 (420 × 297 mm)",
    );
  });

  it("rounds custom dimensions to whole millimetres", () => {
    expect(describeSize(123.4 * PT_PER_MM, 456.6 * PT_PER_MM)).toBe(
      "123 × 457 mm",
    );
  });
});

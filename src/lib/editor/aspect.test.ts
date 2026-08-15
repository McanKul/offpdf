import { describe, it, expect } from "vitest";
import {
  aspectLocked,
  constrainCssBox1to1,
  cssBoxFromPoints,
  isNearlySquare,
  resizeCssRectLocked,
  sizeWithAspect,
} from "./aspect";

describe("constrainCssBox1to1", () => {
  it("grows southeast into a square", () => {
    expect(constrainCssBox1to1({ x: 10, y: 20 }, { x: 50, y: 30 })).toEqual({
      x: 10,
      y: 20,
      w: 40,
      h: 40,
    });
  });

  it("keeps the start corner when dragging northwest", () => {
    expect(constrainCssBox1to1({ x: 80, y: 80 }, { x: 50, y: 40 })).toEqual({
      x: 40,
      y: 40,
      w: 40,
      h: 40,
    });
  });
});

describe("cssBoxFromPoints", () => {
  it("normalizes any two corners", () => {
    expect(cssBoxFromPoints({ x: 30, y: 10 }, { x: 10, y: 40 })).toEqual({
      x: 10,
      y: 10,
      w: 20,
      h: 30,
    });
  });
});

describe("resizeCssRectLocked", () => {
  const start = { x: 10, y: 20, w: 40, h: 20 };

  it("se keeps 2:1 and the top-left", () => {
    const next = resizeCssRectLocked(start, "se", 20, 5);
    expect(next.x).toBeCloseTo(10);
    expect(next.y).toBeCloseTo(20);
    expect(next.w / next.h).toBeCloseTo(2);
    expect(next.w).toBeGreaterThan(40);
  });

  it("nw keeps the bottom-right", () => {
    const next = resizeCssRectLocked(start, "nw", -10, -10);
    expect(next.x + next.w).toBeCloseTo(50);
    expect(next.y + next.h).toBeCloseTo(40);
    expect(next.w / next.h).toBeCloseTo(2);
  });
});

describe("aspectLocked", () => {
  it("locks images by default and Shift unlocks", () => {
    expect(aspectLocked({ kind: "image" }, false)).toBe(true);
    expect(aspectLocked({ kind: "image" }, true)).toBe(false);
  });

  it("locks shapes only when keepAspect is set", () => {
    expect(aspectLocked({ kind: "rect" }, false)).toBe(false);
    expect(aspectLocked({ kind: "rect" }, true)).toBe(true);
    expect(aspectLocked({ kind: "ellipse", keepAspect: true }, false)).toBe(true);
    expect(aspectLocked({ kind: "ellipse", keepAspect: true }, true)).toBe(false);
  });
});

describe("sizeWithAspect", () => {
  const rect = { x: 5, y: 6, w: 40, h: 20 };

  it("updates height when width changes under lock", () => {
    expect(sizeWithAspect(rect, { w: 80 }, true)).toEqual({ x: 5, y: 6, w: 80, h: 40 });
  });

  it("leaves the other side free when unlocked", () => {
    expect(sizeWithAspect(rect, { w: 80 }, false)).toEqual({ x: 5, y: 6, w: 80, h: 20 });
  });
});

describe("isNearlySquare", () => {
  it("treats equal sides as square", () => {
    expect(isNearlySquare({ x: 0, y: 0, w: 50, h: 50 })).toBe(true);
    expect(isNearlySquare({ x: 0, y: 0, w: 80, h: 20 })).toBe(false);
  });
});

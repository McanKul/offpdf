import { describe, it, expect } from "vitest";
import { normalizeDeg, rotateCss, snapDeg } from "./rotate";

describe("object rotation", () => {
  it("round-trips a point 90° around origin", () => {
    const p = rotateCss({ x: 10, y: 0 }, { x: 0, y: 0 }, 90);
    expect(p.x).toBeCloseTo(0, 6);
    expect(p.y).toBeCloseTo(10, 6);
    const back = rotateCss(p, { x: 0, y: 0 }, -90);
    expect(back.x).toBeCloseTo(10, 6);
    expect(back.y).toBeCloseTo(0, 6);
  });

  it("normalizes and snaps angles", () => {
    expect(normalizeDeg(190)).toBe(-170);
    expect(snapDeg(22)).toBe(15);
    expect(snapDeg(23)).toBe(30);
  });
});

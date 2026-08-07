import { describe, it, expect } from "vitest";
import { closedShapeCssPoints, starPoints } from "./shapes";

describe("closed shapes", () => {
  it("triangle has three points, tip at top", () => {
    const s = closedShapeCssPoints("triangle", { x: 0, y: 0, w: 10, h: 10 });
    expect(s.split(" ")).toHaveLength(3);
    expect(s.startsWith("5,0")).toBe(true);
  });

  it("star has 10 alternating vertices", () => {
    const pts = starPoints(0, 0, 10, 10, true);
    expect(pts).toHaveLength(10);
    expect(pts[0][1]).toBeLessThan(0);
  });

  it("hexagon and arrow have the expected vertex counts", () => {
    expect(closedShapeCssPoints("hexagon", { x: 0, y: 0, w: 20, h: 20 }).split(" ")).toHaveLength(6);
    expect(closedShapeCssPoints("arrow", { x: 0, y: 0, w: 20, h: 10 }).split(" ")).toHaveLength(7);
  });
});

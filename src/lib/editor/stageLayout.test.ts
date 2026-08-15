import { describe, it, expect } from "vitest";
import { stageJustify } from "./stageLayout";

describe("stageJustify", () => {
  it("Z1: centers when the page is narrower than the stage", () => {
    expect(stageJustify(612, 900)).toBe("center");
  });

  it("Z1: starts when the page is wider than the stage so the left edge is reachable", () => {
    // Letter at MAX_ZOOM 4 → 2448 CSS px vs a typical stage width.
    expect(stageJustify(612 * 4, 900)).toBe("start");
  });
});

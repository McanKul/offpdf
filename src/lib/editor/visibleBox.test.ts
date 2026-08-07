import { describe, it, expect } from "vitest";
import { visiblePageBox, quadToBox } from "./visibleBox";

describe("visiblePageBox", () => {
  it("returns MediaBox when CropBox is missing", () => {
    const b = visiblePageBox([0, 0, 612, 792]);
    expect(b).toEqual({ x: 0, y: 0, w: 612, h: 792 });
  });

  it("intersects CropBox with MediaBox", () => {
    const b = visiblePageBox([0, 0, 612, 792], [72, 72, 540, 720]);
    expect(b).toEqual({ x: 72, y: 72, w: 468, h: 648 });
  });

  it("clips CropBox that extends outside MediaBox", () => {
    const b = visiblePageBox([0, 0, 200, 200], [-10, -10, 250, 180]);
    expect(b).toEqual({ x: 0, y: 0, w: 200, h: 180 });
  });

  it("falls back to MediaBox when intersection is tiny", () => {
    const b = visiblePageBox([0, 0, 612, 792], [0, 0, 0.5, 0.5]);
    expect(b).toEqual({ x: 0, y: 0, w: 612, h: 792 });
  });

  it("normalizes inverted quads", () => {
    expect(quadToBox([612, 792, 0, 0])).toEqual({ x: 0, y: 0, w: 612, h: 792 });
  });
});

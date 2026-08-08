import { describe, it, expect } from "vitest";
import { visiblePageBox, alignPageBox, quadToBox } from "./visibleBox";

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

describe("alignPageBox", () => {
  const media: [number, number, number, number] = [0, 0, 612, 792];

  it("prefers TrimBox over CropBox", () => {
    const b = alignPageBox(media, [72, 72, 540, 720], [0, 0, 612, 792]);
    expect(b).toEqual({ x: 0, y: 0, w: 612, h: 792 });
  });

  it("falls back to CropBox when Trim is missing", () => {
    const b = alignPageBox(media, [72, 72, 540, 720], null);
    expect(b).toEqual({ x: 72, y: 72, w: 468, h: 648 });
  });

  it("falls back to MediaBox when both missing", () => {
    expect(alignPageBox(media)).toEqual({ x: 0, y: 0, w: 612, h: 792 });
  });

  it("uses raw Trim even if it pokes outside MediaBox (qpdf getTrimBox)", () => {
    const b = alignPageBox(media, null, [-10, -10, 700, 700]);
    expect(b).toEqual({ x: -10, y: -10, w: 710, h: 710 });
  });
});

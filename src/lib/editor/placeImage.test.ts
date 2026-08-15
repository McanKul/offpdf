import { describe, it, expect } from "vitest";
import { imageCssRect, placeImagePdfRect } from "./placeImage";
import { displayedSize } from "./coords";
import type { PageGeometry, PageRotation } from "./types";

const letter = (rotate: PageRotation): PageGeometry => ({
  box: { x: 0, y: 0, w: 612, h: 792 },
  rotate,
  pageIndex: 0,
});

describe("imageCssRect", () => {
  it("centers a landscape image on an unrotated page", () => {
    const css = imageCssRect(200, 100, 612, 792, 612, 792);
    expect(css.w).toBeCloseTo(200);
    expect(css.h).toBeCloseTo(100);
    expect(css.x).toBeCloseTo((612 - 200) / 2);
    expect(css.y).toBeCloseTo((792 - 100) / 2);
  });

  it("uses displayed size on 90° so aspect matches preview", () => {
    const g = letter(90);
    const disp = displayedSize(g);
    expect(disp).toEqual({ w: 792, h: 612 });
    const css = imageCssRect(200, 100, 792, 612, disp.w, disp.h);
    expect(css.w).toBeCloseTo(200);
    expect(css.h).toBeCloseTo(100);
    expect(css.w / css.h).toBeCloseTo(2);
  });
});

describe("placeImagePdfRect", () => {
  it("round-trips a centered image at rotate 0", () => {
    const r = placeImagePdfRect(letter(0), 612, 792, 200, 100);
    expect(r.w).toBeCloseTo(200, 5);
    expect(r.h).toBeCloseTo(100, 5);
  });

  it("stores swapped axes at rotate 90 so export AABB is displayed 200×100", () => {
    const r = placeImagePdfRect(letter(90), 792, 612, 200, 100);
    expect(r.w).toBeCloseTo(100, 4);
    expect(r.h).toBeCloseTo(200, 4);
  });
});

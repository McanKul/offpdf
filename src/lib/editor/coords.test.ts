import { describe, it, expect } from "vitest";
import {
  displayedSize,
  unrotatedToDisplay,
  displayToUnrotated,
  pdfToViewport,
  viewportToPdf,
  pdfRectToViewport,
  viewportRectToPdf,
  makeMapping,
} from "./coords";
import type { PageGeometry, PageRotation, PdfRect } from "./types";

const letter: PageGeometry = {
  box: { x: 0, y: 0, w: 612, h: 792 },
  rotate: 0,
  pageIndex: 0,
};

const cropped: PageGeometry = {
  box: { x: 72, y: 72, w: 400, h: 500 },
  rotate: 0,
  pageIndex: 0,
};

function geom(box: PageGeometry["box"], rotate: PageRotation): PageGeometry {
  return { box, rotate, pageIndex: 0 };
}

function approx(a: number, b: number, eps = 1e-6) {
  expect(Math.abs(a - b)).toBeLessThanOrEqual(eps);
}

function approxPoint(
  p: { x: number; y: number },
  q: { x: number; y: number },
  eps = 1e-6,
) {
  approx(p.x, q.x, eps);
  approx(p.y, q.y, eps);
}

describe("displayedSize", () => {
  it("keeps size for 0 and 180", () => {
    expect(displayedSize(geom(letter.box, 0))).toEqual({ w: 612, h: 792 });
    expect(displayedSize(geom(letter.box, 180))).toEqual({ w: 612, h: 792 });
  });

  it("swaps size for 90 and 270", () => {
    expect(displayedSize(geom(letter.box, 90))).toEqual({ w: 792, h: 612 });
    expect(displayedSize(geom(letter.box, 270))).toEqual({ w: 792, h: 612 });
  });
});

describe("unrotatedToDisplay / displayToUnrotated", () => {
  const corners = [
    { x: 0, y: 0 },
    { x: 612, y: 0 },
    { x: 612, y: 792 },
    { x: 0, y: 792 },
  ];

  for (const rotate of [0, 90, 180, 270] as PageRotation[]) {
    it(`round-trips relative corners at ${rotate}°`, () => {
      for (const c of corners) {
        const d = unrotatedToDisplay(c.x, c.y, 612, 792, rotate);
        const back = displayToUnrotated(d.x, d.y, 612, 792, rotate);
        approxPoint(back, c);
      }
    });
  }

  it("maps 90° CW corners correctly", () => {
    // BL → top of display (y = boxW), x = 0
    expect(unrotatedToDisplay(0, 0, 612, 792, 90)).toEqual({ x: 0, y: 612 });
    // BR → BL of display
    expect(unrotatedToDisplay(612, 0, 612, 792, 90)).toEqual({ x: 0, y: 0 });
  });
});

describe("pdfToViewport / viewportToPdf (rotate 0)", () => {
  const m = makeMapping(letter, 612, 792);

  it("maps page corners to CSS corners", () => {
    approxPoint(pdfToViewport({ x: 0, y: 792 }, m), { x: 0, y: 0 }); // TL
    approxPoint(pdfToViewport({ x: 612, y: 792 }, m), { x: 612, y: 0 }); // TR
    approxPoint(pdfToViewport({ x: 0, y: 0 }, m), { x: 0, y: 792 }); // BL
    approxPoint(pdfToViewport({ x: 612, y: 0 }, m), { x: 612, y: 792 }); // BR
  });

  it("round-trips interior points", () => {
    const pts = [
      { x: 100, y: 200 },
      { x: 300, y: 400 },
      { x: 50.5, y: 700.25 },
    ];
    for (const p of pts) {
      approxPoint(viewportToPdf(pdfToViewport(p, m), m), p);
    }
  });
});

describe("zoom / CSS size scaling", () => {
  it("doubling CSS size maps the same relative pointer to the same PDF point", () => {
    const g = letter;
    const m1 = makeMapping(g, 306, 396);
    const m2 = makeMapping(g, 612, 792);
    // Center of the page in CSS for both mappings
    const pdf1 = viewportToPdf({ x: 153, y: 198 }, m1);
    const pdf2 = viewportToPdf({ x: 306, y: 396 }, m2);
    approxPoint(pdf1, pdf2);
    approxPoint(pdf1, { x: 306, y: 396 });
  });

  it("object PDF coords are independent of zoom (export stability)", () => {
    const rect: PdfRect = { x: 100, y: 200, w: 50, h: 40 };
    // PDF coords are the source of truth — zoom only changes viewport mapping
    const m1 = makeMapping(letter, 306, 396);
    const m2 = makeMapping(letter, 612, 792);
    const css1 = pdfRectToViewport(rect, m1);
    const css2 = pdfRectToViewport(rect, m2);
    // CSS doubles; recovering PDF yields the same rect
    const back1 = viewportRectToPdf(css1, m1);
    const back2 = viewportRectToPdf(css2, m2);
    approx(back1.x, rect.x);
    approx(back1.y, rect.y);
    approx(back1.w, rect.w);
    approx(back1.h, rect.h);
    approx(back2.x, rect.x);
    approx(back2.y, rect.y);
    approx(back2.w, rect.w);
    approx(back2.h, rect.h);
  });
});

describe("CropBox offset", () => {
  it("maps relative to non-zero box origin", () => {
    const m = makeMapping(cropped, 400, 500);
    // Lower-left of crop box
    approxPoint(pdfToViewport({ x: 72, y: 72 }, m), { x: 0, y: 500 });
    // Upper-left of crop box
    approxPoint(pdfToViewport({ x: 72, y: 572 }, m), { x: 0, y: 0 });
    // Round-trip an interior point
    const p = { x: 172, y: 272 };
    approxPoint(viewportToPdf(pdfToViewport(p, m), m), p);
  });
});

describe("rotation: PDF export coords stable across display rotation", () => {
  const known = { x: 100, y: 200 };

  for (const rotate of [0, 90, 180, 270] as PageRotation[]) {
    it(`round-trips known point at page rotate ${rotate}°`, () => {
      const g = geom(letter.box, rotate);
      const size = displayedSize(g);
      // Use CSS size proportional to displayed size
      const m = makeMapping(g, size.w, size.h);
      const css = pdfToViewport(known, m);
      const back = viewportToPdf(css, m);
      approxPoint(back, known);
    });
  }

  it("same PDF point maps to different CSS under different rotations", () => {
    const m0 = makeMapping(geom(letter.box, 0), 612, 792);
    const m90 = makeMapping(geom(letter.box, 90), 792, 612);
    const c0 = pdfToViewport(known, m0);
    const c90 = pdfToViewport(known, m90);
    // Not the same pixel location when page is rotated
    expect(c0.x !== c90.x || c0.y !== c90.y).toBe(true);
    // But both recover the same PDF export coordinate
    approxPoint(viewportToPdf(c0, m0), known);
    approxPoint(viewportToPdf(c90, m90), known);
  });
});

describe("pdfRectToViewport / viewportRectToPdf", () => {
  it("round-trips a rect at 0°", () => {
    const m = makeMapping(letter, 612, 792);
    const rect: PdfRect = { x: 50, y: 100, w: 80, h: 60 };
    const css = pdfRectToViewport(rect, m);
    const back = viewportRectToPdf(css, m);
    approx(back.x, rect.x);
    approx(back.y, rect.y);
    approx(back.w, rect.w);
    approx(back.h, rect.h);
  });

  it("round-trips a rect at 90°", () => {
    const m = makeMapping(geom(letter.box, 90), 792, 612);
    const rect: PdfRect = { x: 50, y: 100, w: 80, h: 60 };
    const css = pdfRectToViewport(rect, m);
    const back = viewportRectToPdf(css, m);
    approx(back.x, rect.x, 1e-5);
    approx(back.y, rect.y, 1e-5);
    approx(back.w, rect.w, 1e-5);
    approx(back.h, rect.h, 1e-5);
  });
});

import { describe, it, expect } from "vitest";
import { resizePdfRect } from "./resizeRect";
import type { PdfRect } from "./types";

const start: PdfRect = { x: 100, y: 200, w: 80, h: 60 };
// Corners: LL(100,200) LR(180,200) UL(100,260) UR(180,260)

function approxRect(a: PdfRect, b: PdfRect, eps = 1e-9) {
  expect(Math.abs(a.x - b.x)).toBeLessThanOrEqual(eps);
  expect(Math.abs(a.y - b.y)).toBeLessThanOrEqual(eps);
  expect(Math.abs(a.w - b.w)).toBeLessThanOrEqual(eps);
  expect(Math.abs(a.h - b.h)).toBeLessThanOrEqual(eps);
}

describe("resizePdfRect (PDF y-up, handle names are CSS)", () => {
  it("sw (bottom-left / PDF LL): opposite UR stays fixed", () => {
    // Drag LL left 10 and down 5 in PDF (dy negative = toward bottom of page)
    const next = resizePdfRect(start, "sw", -10, -5);
    // UR = (x+w, y+h) must stay (180, 260)
    expect(next.x + next.w).toBeCloseTo(180);
    expect(next.y + next.h).toBeCloseTo(260);
    approxRect(next, { x: 90, y: 195, w: 90, h: 65 });
  });

  it("se (bottom-right / PDF LR): opposite UL stays fixed", () => {
    const next = resizePdfRect(start, "se", 10, -5);
    // UL = (x, y+h) must stay (100, 260)
    expect(next.x).toBeCloseTo(100);
    expect(next.y + next.h).toBeCloseTo(260);
    approxRect(next, { x: 100, y: 195, w: 90, h: 65 });
  });

  it("ne (top-right / PDF UR): opposite LL stays fixed", () => {
    const next = resizePdfRect(start, "ne", 10, 5);
    expect(next.x).toBeCloseTo(100);
    expect(next.y).toBeCloseTo(200);
    approxRect(next, { x: 100, y: 200, w: 90, h: 65 });
  });

  it("nw (top-left / PDF UL): opposite LR stays fixed", () => {
    const next = resizePdfRect(start, "nw", -10, 5);
    // LR = (x+w, y) must stay (180, 200)
    expect(next.x + next.w).toBeCloseTo(180);
    expect(next.y).toBeCloseTo(200);
    approxRect(next, { x: 90, y: 200, w: 90, h: 65 });
  });

  it("dragging sw further bottom-left does not pin top-left", () => {
    // Regression: old CSS-style math kept top edge wrong by treating y as top-left.
    const next = resizePdfRect(start, "sw", -20, -30);
    // Top-left in CSS = PDF UL (x, y+h) should move only in x, not stay as visual pivot wrongly
    // Fixed is UR
    expect(next.x + next.w).toBeCloseTo(180);
    expect(next.y + next.h).toBeCloseTo(260);
    // LL moved
    expect(next.x).toBeCloseTo(80);
    expect(next.y).toBeCloseTo(170);
  });
});

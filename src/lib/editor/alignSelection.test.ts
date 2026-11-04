import { describe, it, expect } from "vitest";
import {
  ALIGN_COMMANDS,
  ALIGN_LABELS,
  alignSelectedObjects,
  displayBounds,
  displayDeltaToPdf,
  isAlignableObject,
  type AlignCommand,
} from "./alignSelection";
import {
  canRedo,
  canUndo,
  createHistoryState,
  editReducer,
  makeRectObject,
} from "./editReducer";
import { displayedSize } from "./coords";
import type { EditDocument, EditObject, PageGeometry, PageRotation, PdfRect } from "./types";

function geom(box: PdfRect, rotate: PageRotation = 0): PageGeometry {
  return { box, rotate, pageIndex: 0 };
}

function doc(objects: EditObject[], selectedIds: string[]): EditDocument {
  return { version: 1, objects, selectedIds };
}

function run(
  objects: EditObject[],
  selectedIds: string[],
  geometry: PageGeometry,
  command: AlignCommand,
) {
  const result = alignSelectedObjects(objects, selectedIds, geometry.pageIndex, geometry, command);
  expect(result).not.toBeNull();
  return result!;
}

function rectOf(objects: EditObject[], id: string): PdfRect {
  const object = objects.find((candidate) => candidate.id === id);
  expect(object).toBeDefined();
  return object!.rect;
}

function approx(a: number, b: number, eps = 1e-6) {
  expect(Math.abs(a - b)).toBeLessThanOrEqual(eps);
}

describe("isAlignableObject", () => {
  it("moves normal objects, not locked objects or text changes", () => {
    const plain = makeRectObject("a", 0, { x: 0, y: 0, w: 10, h: 10 });
    expect(isAlignableObject(plain)).toBe(true);
    expect(isAlignableObject({ ...plain, locked: true })).toBe(false);
    expect(
      isAlignableObject({
        ...plain,
        kind: "sourceText",
        locked: true,
      } as unknown as EditObject),
    ).toBe(false);
  });
});

describe("alignSelectedObjects: one object uses the visible page box", () => {
  const geometry = geom({ x: 50, y: 100, w: 200, h: 300 });

  const cases: { command: AlignCommand; x: number; y: number }[] = [
    { command: "left", x: 50, y: 120 },
    { command: "horizontalCenter", x: 130, y: 120 },
    { command: "right", x: 210, y: 120 },
    { command: "top", x: 60, y: 380 },
    { command: "verticalCenter", x: 60, y: 240 },
    { command: "bottom", x: 60, y: 100 },
  ];

  for (const expected of cases) {
    it(`${expected.command} includes the CropBox offset`, () => {
      const object = makeRectObject("a", 0, { x: 60, y: 120, w: 40, h: 20 });
      const result = run([object], ["a"], geometry, expected.command);
      expect(rectOf(result.objects, "a")).toMatchObject({
        x: expected.x,
        y: expected.y,
        w: 40,
        h: 20,
      });
    });
  }
});

describe("alignSelectedObjects: multi-select uses mixed-size selection bounds", () => {
  const geometry = geom({ x: 0, y: 0, w: 500, h: 500 });
  const a = makeRectObject("a", 0, { x: 10, y: 10, w: 20, h: 10 });
  const b = makeRectObject("b", 0, { x: 100, y: 50, w: 60, h: 40 });
  const locked = { ...makeRectObject("locked", 0, { x: 200, y: 200, w: 10, h: 10 }), locked: true };

  const cases: { command: AlignCommand; a: { x: number; y: number }; b: { x: number; y: number } }[] = [
    { command: "left", a: { x: 10, y: 10 }, b: { x: 10, y: 50 } },
    { command: "horizontalCenter", a: { x: 100, y: 10 }, b: { x: 80, y: 50 } },
    { command: "right", a: { x: 190, y: 10 }, b: { x: 150, y: 50 } },
    { command: "top", a: { x: 10, y: 200 }, b: { x: 100, y: 170 } },
    { command: "verticalCenter", a: { x: 10, y: 105 }, b: { x: 100, y: 90 } },
    { command: "bottom", a: { x: 10, y: 10 }, b: { x: 100, y: 10 } },
  ];

  for (const expected of cases) {
    it(`${expected.command} keeps the locked object and every size`, () => {
      const result = run([a, b, locked], ["a", "b", "locked"], geometry, expected.command);
      expect(rectOf(result.objects, "a")).toMatchObject(expected.a);
      expect(rectOf(result.objects, "b")).toMatchObject(expected.b);
      expect(rectOf(result.objects, "locked")).toEqual(locked.rect);
    });
  }

  it("leaves objects on another page unchanged", () => {
    const hidden = makeRectObject("hidden", 1, { x: 300, y: 300, w: 30, h: 30 });
    const result = run([a, hidden], ["a", "hidden"], geometry, "left");
    expect(rectOf(result.objects, "hidden")).toEqual(hidden.rect);
    expect(result.changedIds).toEqual(["a"]);
  });

  it("returns null when only locked objects are selected", () => {
    expect(alignSelectedObjects([locked], ["locked"], 0, geometry, "left")).toBeNull();
  });
});

describe("alignSelectedObjects: page bounds", () => {
  it("clamps an alignment target that would leave the visible page", () => {
    const geometry = geom({ x: 0, y: 0, w: 100, h: 100 });
    const a = makeRectObject("a", 0, { x: 10, y: 10, w: 20, h: 20 });
    const b = makeRectObject("b", 0, { x: 90, y: 10, w: 30, h: 20 });

    const result = run([a, b], ["a", "b"], geometry, "right");
    expect(rectOf(result.objects, "a").x).toBe(80);
    expect(rectOf(result.objects, "b").x).toBe(70);
    expect(rectOf(result.objects, "a").x + 20).toBeLessThanOrEqual(100);
    expect(rectOf(result.objects, "b").x + 30).toBeLessThanOrEqual(100);
  });

  it("keeps an object wider than the page at the page origin", () => {
    const geometry = geom({ x: 0, y: 0, w: 100, h: 100 });
    const big = makeRectObject("big", 0, { x: -10, y: 0, w: 150, h: 20 });
    const result = run([big], ["big"], geometry, "horizontalCenter");
    expect(rectOf(result.objects, "big").x).toBe(0);
  });

  it("does not create a history step when the command changes nothing", () => {
    const geometry = geom({ x: 0, y: 0, w: 100, h: 100 });
    const object = makeRectObject("a", 0, { x: 0, y: 20, w: 20, h: 20 });
    let state = createHistoryState(doc([object], ["a"]));
    state = editReducer(state, { type: "ALIGN", command: "left", pageIndex: 0, geometry });
    expect(state.present.objects[0].rect).toEqual(object.rect);
    expect(canUndo(state)).toBe(false);
  });
});

describe("alignSelectedObjects: page rotation", () => {
  const rotations: PageRotation[] = [0, 90, 180, 270];

  for (const rotate of rotations) {
    it(`aligns to displayed edges at ${rotate} degrees`, () => {
      const geometry = geom({ x: 100, y: 200, w: 200, h: 100 }, rotate);
      const page = displayedSize(geometry);
      const start = makeRectObject("a", 0, { x: 110, y: 220, w: 20, h: 40 });

      const checks: { command: AlignCommand; check: (box: PdfRect) => void }[] = [
        { command: "left", check: (box) => approx(box.x, 0) },
        { command: "horizontalCenter", check: (box) => approx(box.x + box.w / 2, page.w / 2) },
        { command: "right", check: (box) => approx(box.x + box.w, page.w) },
        { command: "top", check: (box) => approx(box.y + box.h, page.h) },
        { command: "verticalCenter", check: (box) => approx(box.y + box.h / 2, page.h / 2) },
        { command: "bottom", check: (box) => approx(box.y, 0) },
      ];

      for (const { command, check } of checks) {
        const result = run([start], ["a"], geometry, command);
        check(displayBounds(rectOf(result.objects, "a"), geometry));
      }
    });
  }

  it("maps display deltas back to PDF deltas", () => {
    expect(displayDeltaToPdf(10, -4, 0)).toEqual({ x: 10, y: -4 });
    expect(displayDeltaToPdf(10, -4, 90)).toEqual({ x: 4, y: 10 });
    expect(displayDeltaToPdf(10, -4, 180)).toEqual({ x: -10, y: 4 });
    expect(displayDeltaToPdf(10, -4, 270)).toEqual({ x: -4, y: -10 });
  });
});

describe("alignSelectedObjects: rotated editor objects", () => {
  const rotations: PageRotation[] = [0, 90, 180, 270];

  it("uses the AABB of the object's own SVG rotation", () => {
    const bounds = displayBounds(
      { x: 40, y: 50, w: 40, h: 20 },
      geom({ x: 0, y: 0, w: 200, h: 200 }),
      45,
    );
    approx(bounds.w, 30 * Math.SQRT2);
    approx(bounds.h, 30 * Math.SQRT2);
    approx(bounds.x + bounds.w / 2, 60);
    approx(bounds.y + bounds.h / 2, 60);
  });

  for (const rotate of rotations) {
    it(`aligns the visible edges of a 45° object on a ${rotate}° page`, () => {
      const geometry = geom({ x: 50, y: 100, w: 200, h: 300 }, rotate);
      const page = displayedSize(geometry);
      const object = { ...makeRectObject("a", 0, { x: 80, y: 150, w: 40, h: 20 }), objectRotate: 45 };

      for (const command of ALIGN_COMMANDS) {
        const result = run([object], ["a"], geometry, command);
        const visual = displayBounds(rectOf(result.objects, "a"), geometry, object.objectRotate);
        if (command === "left") approx(visual.x, 0);
        if (command === "horizontalCenter") approx(visual.x + visual.w / 2, page.w / 2);
        if (command === "right") approx(visual.x + visual.w, page.w);
        if (command === "top") approx(visual.y + visual.h, page.h);
        if (command === "verticalCenter") approx(visual.y + visual.h / 2, page.h / 2);
        if (command === "bottom") approx(visual.y, 0);
        if (command === "left" || command === "horizontalCenter" || command === "right") {
          expect(visual.x).toBeGreaterThanOrEqual(-1e-6);
          expect(visual.x + visual.w).toBeLessThanOrEqual(page.w + 1e-6);
        } else {
          expect(visual.y).toBeGreaterThanOrEqual(-1e-6);
          expect(visual.y + visual.h).toBeLessThanOrEqual(page.h + 1e-6);
        }
      }
    });
  }

  it("aligns a rotated and an unrotated object's visible left edges", () => {
    const geometry = geom({ x: 0, y: 0, w: 300, h: 300 });
    const rotated = { ...makeRectObject("rotated", 0, { x: 40, y: 40, w: 40, h: 20 }), objectRotate: 45 };
    const plain = makeRectObject("plain", 0, { x: 150, y: 80, w: 20, h: 20 });
    const result = run([rotated, plain], ["rotated", "plain"], geometry, "left");
    const rotatedBounds = displayBounds(rectOf(result.objects, "rotated"), geometry, 45);
    const plainBounds = displayBounds(rectOf(result.objects, "plain"), geometry);
    approx(rotatedBounds.x, plainBounds.x);
  });
});

describe("ALIGN command history", () => {
  it("is one undo/redo step and keeps the multi-selection", () => {
    const geometry = geom({ x: 0, y: 0, w: 500, h: 500 });
    const a = makeRectObject("a", 0, { x: 10, y: 10, w: 20, h: 10 });
    const b = makeRectObject("b", 0, { x: 100, y: 50, w: 60, h: 40 });
    let state = createHistoryState(doc([a, b], ["a", "b"]));

    state = editReducer(state, { type: "ALIGN", command: "left", pageIndex: 0, geometry });
    expect(state.past).toHaveLength(1);
    expect(state.present.selectedIds).toEqual(["a", "b"]);
    expect(rectOf(state.present.objects, "a").x).toBe(10);
    expect(rectOf(state.present.objects, "b").x).toBe(10);
    expect(canUndo(state)).toBe(true);

    state = editReducer(state, { type: "UNDO" });
    expect(rectOf(state.present.objects, "a").x).toBe(10);
    expect(rectOf(state.present.objects, "b").x).toBe(100);
    expect(canRedo(state)).toBe(true);

    state = editReducer(state, { type: "REDO" });
    expect(rectOf(state.present.objects, "b").x).toBe(10);
    expect(state.past).toHaveLength(1);
  });

  it("labels every command", () => {
    for (const command of ALIGN_COMMANDS) {
      expect(ALIGN_LABELS[command].length).toBeGreaterThan(0);
    }
  });
});

import { describe, expect, it } from "vitest";
import type { TextEditVerdict, TextFont, TextRun } from "../types";
import { displayedSize, makeMapping, pdfRectToViewport } from "./coords";
import type { PageGeometry } from "./types";
import { makeSourceTextObject } from "./editReducer";
import {
  blockingProblem,
  blockingVerdict,
  caretIndexAt,
  editedGeometry,
  editsSignature,
  estimateCaretOffsets,
  estimateDeltaPt,
  fontsByKey,
  isNoOpEdit,
  missingChars,
  normaliseStyle,
  normaliseTyped,
  overlapsNext,
  problemMessage,
  readingNeighbour,
  spaceProblem,
  surfaceFor,
  TEXT_INKS,
  textStampGeometry,
  toTextEditIn,
} from "./sourceText";
import { REASON_COPY, UI } from "./sourceTextCopy";

// Widths in thousandths (as `/Widths`). "Hello" = 722 + 556 + 222 + 222 + 556 = 2278.
const F1: TextFont = {
  key: "f1",
  displayName: "Helvetica",
  familyHint: "sans",
  embedded: true,
  subset: true,
  alphabet: " HKelo!",
  widths: [278, 722, 667, 556, 222, 556, 278],
  wordSpace: true,
};
/** Word-style Identity-H companion carrying the Turkish letters. */
const F2: TextFont = { ...F1, key: "f2", alphabet: "ğşı", widths: [556, 500, 278], wordSpace: false };
const FB: TextFont = { ...F1, key: "fb", displayName: "Helvetica-Bold", alphabet: " Helo", widths: [600, 600, 600, 600, 600] };
/** pdfTeX-style font: no space glyph. */
const FT: TextFont = { ...F1, key: "ft", subset: false, alphabet: "ab", widths: [500, 500] };
const FONTS = fontsByKey([F1, F2, FB, FT]);

const HELLO_WIDTH = 27.336; // 2278 / 1000 × 12

function run(overrides: Partial<TextRun> = {}, metrics: Partial<NonNullable<TextRun["metrics"]>> = {}): TextRun {
  const base: TextRun = {
    id: "t1:fp:0:10-20",
    order: 0,
    line: 0,
    text: "Hello",
    rect: { x: 72, y: 697, w: HELLO_WIDTH, h: 12 },
    origin: { x: 72, y: 700 },
    dir: { x: 1, y: 0 },
    ascent: 9,
    descent: 3,
    caretOffsets: [0, 8.664, 15.336, 18, 20.664, HELLO_WIDTH],
    editable: true,
    reason: null,
    metrics: {
      surface: ["f1", "f2"],
      tfSize: 12,
      effectiveSize: 12,
      charSpacing: 0,
      wordSpacing: 0,
      hScale: 1,
      textToUser: 1,
      letterSpacingPt: 0,
      spaceMode: "glyph",
      kernSpace: -250,
      originalWidth: HELLO_WIDTH,
      visibleExtent: 100,
      nextObstacle: null,
      ...metrics,
    },
    style: {
      fill: "#000000",
      sizeChangeable: true,
      colourChangeable: true,
      face: "regular",
      faces: {
        regular: { available: true, surface: ["f1", "f2"] },
        bold: { available: true, surface: ["fb"] },
        italic: { available: false, surface: [] },
        boldItalic: { available: false, surface: [] },
      },
    },
    substituted: false,
  };
  return { ...base, ...overrides };
}

function kernRun(text = "ab"): TextRun {
  return run({ text, caretOffsets: [0, 6, 12] }, { surface: ["ft"], spaceMode: "kern", originalWidth: 12 });
}

function refused(): TextRun {
  return run({ editable: false, reason: "SHARED_CONTENT", metrics: null, style: null });
}

function verdict(overrides: Partial<TextEditVerdict>): TextEditVerdict {
  return {
    runId: "t1:fp:0:10-20",
    ok: false,
    code: null,
    chars: [],
    reason: null,
    face: null,
    field: null,
    detail: null,
    deltaPt: 0,
    newRect: null,
    caretOffsets: null,
    ...overrides,
  };
}

const COMBINING_BREVE = String.fromCharCode(0x0306);
const LINE_SEPARATOR = String.fromCharCode(0x2028);

describe("normaliseTyped", () => {
  it("composes to NFC (g + combining breve → ğ)", () => {
    expect(normaliseTyped(`g${COMBINING_BREVE}`)).toBe("ğ");
    expect(normaliseTyped("ğ")).toBe("ğ");
  });
});

describe("missingChars", () => {
  it("deduplicates in typing order", () => {
    expect(missingChars(run(), FONTS, "Hxzxqz")).toEqual(["x", "z", "q"]);
  });

  it("routes letters through sibling fonts of the surface", () => {
    expect(missingChars(run(), FONTS, "Heğlo şı")).toEqual([]);
  });

  it("uses the requested face's surface", () => {
    expect(surfaceFor(run(), "bold")).toEqual(["fb"]);
    expect(missingChars(run(), FONTS, "Heğlo", "bold")).toEqual(["ğ"]);
    expect(surfaceFor(run(), "italic")).toEqual([]);
    expect(missingChars(run(), FONTS, "He", "italic")).toEqual(["H", "e"]);
  });

  it("checks decomposed input after NFC", () => {
    expect(missingChars(run(), FONTS, `He g${COMBINING_BREVE}`)).toEqual([]);
  });

  it("lists a space the glyph-mode surface can't draw, never one in kern mode", () => {
    const noSpace = run({}, { surface: ["f2"] });
    expect(missingChars(noSpace, FONTS, "ğ ş")).toEqual([" "]);
    expect(missingChars(kernRun(), FONTS, "a b")).toEqual([]);
  });
});

describe("spaceProblem", () => {
  it("allows one space between two letters in kern mode", () => {
    expect(spaceProblem(kernRun(), "a b")).toBe(false);
    expect(spaceProblem(kernRun(), "ab")).toBe(false);
  });

  it("refuses leading, trailing and doubled spaces in kern mode", () => {
    expect(spaceProblem(kernRun(), " ab")).toBe(true);
    expect(spaceProblem(kernRun(), "ab ")).toBe(true);
    expect(spaceProblem(kernRun(), "a  b")).toBe(true);
  });

  it("never applies to glyph mode", () => {
    expect(spaceProblem(run(), "  He  ")).toBe(false);
  });
});

describe("estimateDeltaPt", () => {
  it("is 0 for the unchanged line", () => {
    expect(estimateDeltaPt(run(), FONTS, "Hello", {})).toBeCloseTo(0, 9);
  });

  it("adds the font width of a typed letter", () => {
    expect(estimateDeltaPt(run(), FONTS, "Helloo", {})).toBeCloseTo(6.672, 9);
    expect(estimateDeltaPt(run(), FONTS, "Hell", {})).toBeCloseTo(-6.672, 9);
  });

  it("measures sibling-font letters with their own widths", () => {
    expect(estimateDeltaPt(run(), FONTS, "Helloı", {})).toBeCloseTo(3.336, 9);
  });

  it("applies Tz and the text-to-user scale", () => {
    expect(estimateDeltaPt(run({}, { hScale: 0.8 }), FONTS, "Helloo", {})).toBeCloseTo(5.3376, 9);
    expect(estimateDeltaPt(run({}, { textToUser: 2, effectiveSize: 24 }), FONTS, "Helloo", {})).toBeCloseTo(13.344, 9);
  });

  it("applies Tc per glyph and Tw to a single-byte space", () => {
    const spaced = run({}, { charSpacing: 0.5, wordSpacing: 2 });
    // "He llo" vs "Hello": one more glyph (space 3.336 + Tc 0.5) plus Tw 2.
    expect(estimateDeltaPt(spaced, FONTS, "He llo", {})).toBeCloseTo(5.836, 9);
  });

  it("writes a kern-mode space as the run's kern", () => {
    expect(estimateDeltaPt(kernRun(), FONTS, "a b", {})).toBeCloseTo(3, 9);
  });

  it("follows size, letter-spacing and face changes", () => {
    expect(estimateDeltaPt(run(), FONTS, "Hello", { sizePt: 24 })).toBeCloseTo(HELLO_WIDTH, 9);
    expect(estimateDeltaPt(run(), FONTS, "Hello", { letterSpacingPt: 1 })).toBeCloseTo(5, 9);
    expect(estimateDeltaPt(run(), FONTS, "Hello", { face: "bold" })).toBeCloseTo(36 - HELLO_WIDTH, 9);
  });

  it("counts a letter the font lacks as half an em", () => {
    expect(estimateDeltaPt(run(), FONTS, "Hellox", {})).toBeCloseTo(6, 9);
  });

  it("is 0 for a refused run", () => {
    expect(estimateDeltaPt(refused(), FONTS, "Anything", {})).toBe(0);
  });
});

describe("estimateCaretOffsets", () => {
  it("has length + 1 entries, starts at 0 and never goes back", () => {
    const offsets = estimateCaretOffsets(run(), FONTS, "Helloğ şı", { letterSpacingPt: -2 });
    expect(offsets).toHaveLength(Array.from("Helloğ şı").length + 1);
    expect(offsets[0]).toBe(0);
    for (let i = 1; i < offsets.length; i++) expect(offsets[i]).toBeGreaterThanOrEqual(offsets[i - 1]);
  });

  it("matches the run's own offsets for the unchanged line", () => {
    const offsets = estimateCaretOffsets(run(), FONTS, "Hello", {});
    run().caretOffsets.forEach((o, i) => expect(offsets[i]).toBeCloseTo(o, 9));
  });

  it("returns zeros of the right length for a refused run", () => {
    expect(estimateCaretOffsets(refused(), FONTS, "ab", {})).toEqual([0, 0, 0]);
  });
});

describe("normaliseStyle / isNoOpEdit (B7)", () => {
  it("drops fields equal to the current value within 0.001", () => {
    const style = { sizePt: 12.0005, face: "regular" as const, fill: "#000000", letterSpacingPt: 0.0004 };
    expect(normaliseStyle(run(), style)).toEqual({});
    expect(isNoOpEdit(run(), "Hello", style)).toBe(true);
  });

  it("keeps real changes and lower-cases the fill", () => {
    expect(normaliseStyle(run(), { sizePt: 13, fill: "#FF0000", face: "bold", letterSpacingPt: 0.5 })).toEqual({
      sizePt: 13,
      fill: "#ff0000",
      face: "bold",
      letterSpacingPt: 0.5,
    });
  });

  it("keeps a fill when the original colour is unreadable", () => {
    const unreadable = run();
    const style = { ...unreadable.style!, fill: null };
    expect(normaliseStyle({ ...unreadable, style }, { fill: "#000000" })).toEqual({ fill: "#000000" });
  });

  it("treats NFC-equal text as unchanged and different text as a change", () => {
    const decomposed = run({ text: "ğ" });
    expect(isNoOpEdit(decomposed, `g${COMBINING_BREVE}`, {})).toBe(true);
    expect(isNoOpEdit(run(), "Hellö", {})).toBe(false);
    expect(isNoOpEdit(run(), "Hello", { sizePt: 13 })).toBe(false);
  });
});

describe("blockingProblem", () => {
  it("refuses a refused run with its reason", () => {
    const v = blockingVerdict(refused(), FONTS, "x", {});
    expect(v?.code).toBe("TEXT_EDIT_REFUSED");
    expect(v?.reason).toBe("SHARED_CONTENT");
  });

  it("refuses line breaks, tabs and separators before anything else", () => {
    expect(blockingProblem(run(), FONTS, "He\nllo", {})).toBe("INVALID_TEXT");
    expect(blockingProblem(run(), FONTS, "He\tllo", {})).toBe("INVALID_TEXT");
    expect(blockingProblem(run(), FONTS, `He${LINE_SEPARATOR}llo`, {})).toBe("INVALID_TEXT");
  });

  it("refuses more than 1,000 characters", () => {
    expect(blockingProblem(run({}, { visibleExtent: 1e6 }), FONTS, "l".repeat(1001), {})).toBe("TEXT_TOO_LONG");
    expect(blockingProblem(run({}, { visibleExtent: 1e6 }), FONTS, "l".repeat(1000), {})).toBeNull();
  });

  it("is null for a no-op", () => {
    expect(blockingVerdict(run(), FONTS, "Hello", { sizePt: 12 })).toBeNull();
  });

  it("lists missing letters", () => {
    const v = blockingVerdict(run(), FONTS, "Hxllxz", {});
    expect(v?.code).toBe("GLYPH_MISSING");
    expect(v?.chars).toEqual(["x", "z"]);
  });

  it("refuses misplaced spaces in kern mode", () => {
    expect(blockingProblem(kernRun(), FONTS, "a  b", {})).toBe("SPACE_NOT_WRITABLE");
  });

  it("reports an unavailable face, or the letters the face can't draw", () => {
    expect(blockingVerdict(run(), FONTS, "Hello", { face: "italic" })).toMatchObject({
      code: "FACE_UNAVAILABLE",
      face: "italic",
      chars: [],
    });
    expect(blockingVerdict(run(), FONTS, "Heğlo", { face: "bold" })).toMatchObject({
      code: "FACE_UNAVAILABLE",
      face: "bold",
      chars: ["ğ"],
    });
  });

  it("checks letters against the requested face only", () => {
    const boldHasX = fontsByKey([F1, F2, { ...FB, alphabet: " Helox", widths: [600, 600, 600, 600, 600, 600] }]);
    expect(blockingProblem(run(), boldHasX, "Hellox", { face: "bold" })).toBeNull();
    expect(blockingProblem(run(), boldHasX, "Hellox", {})).toBe("GLYPH_MISSING");
  });

  it("refuses size and face for an ExtGState font, colour for outlined text", () => {
    const extGState = run();
    const locked = { ...extGState, style: { ...extGState.style!, sizeChangeable: false } };
    expect(blockingVerdict(locked, FONTS, "Hello", { sizePt: 14 })).toMatchObject({ code: "STYLE_UNAVAILABLE", field: "size" });
    expect(blockingVerdict(locked, FONTS, "Hello", { face: "bold" })).toMatchObject({ code: "STYLE_UNAVAILABLE", field: "face" });
    const outlined = { ...extGState, style: { ...extGState.style!, colourChangeable: false } };
    expect(blockingVerdict(outlined, FONTS, "Hello", { fill: "#c71c1c" })).toMatchObject({
      code: "STYLE_UNAVAILABLE",
      field: "colour",
    });
  });

  it("refuses text that runs past the visible page", () => {
    const tight = run({}, { visibleExtent: 40 });
    expect(blockingProblem(tight, FONTS, "Helloo", {})).toBeNull(); // 34.008
    expect(blockingProblem(tight, FONTS, "Hellooo", {})).toBe("TEXT_OUTSIDE_VISIBLE_AREA"); // 40.68
  });
});

describe("overlapsNext", () => {
  it("warns once the new end passes the next run by more than 0.5 pt", () => {
    const r = run({}, { nextObstacle: 40 });
    expect(overlapsNext(r, FONTS, "Helloo", {})).toBe(false);
    expect(overlapsNext(r, FONTS, "Hellooo", {})).toBe(true);
    expect(overlapsNext(run(), FONTS, "Hellooooooooo", {})).toBe(false);
  });
});

describe("caretIndexAt", () => {
  it("picks the nearest boundary of the run's offsets", () => {
    expect(caretIndexAt(run(), { x: 72 + 16, y: 701 })).toBe(2);
    expect(caretIndexAt(run(), { x: 60, y: 700 })).toBe(0);
    expect(caretIndexAt(run(), { x: 400, y: 700 })).toBe(5);
  });

  it("uses explicit offsets when given", () => {
    expect(caretIndexAt(run(), { x: 72 + 14, y: 700 }, [0, 10, 20])).toBe(1);
    expect(caretIndexAt(run(), { x: 72 + 14, y: 700 }, [])).toBe(0);
  });

  it("projects onto a non-horizontal baseline", () => {
    const vertical = run({ dir: { x: 0, y: -1 } });
    expect(caretIndexAt(vertical, { x: 72, y: 700 - 19 })).toBe(3);
  });
});

describe("editedGeometry", () => {
  const obj = makeSourceTextObject("o1", 0, { x: 72, y: 697, w: 40, h: 12 }, {
    runId: "t1:fp:0:10-20",
    sourceFingerprint: "fp",
    sourcePageIndex: 0,
    originalText: "Hello",
    text: "Helloo",
    style: {},
  });

  it("prefers the current verdict's box and offsets", () => {
    const current = verdict({ ok: true, newRect: { x: 72, y: 697, w: 34, h: 12 }, caretOffsets: [0, 1, 2, 3, 4, 5, 34] });
    expect(editedGeometry(run(), obj, current, FONTS)).toEqual({
      rect: { x: 72, y: 697, w: 34, h: 12 },
      caretOffsets: [0, 1, 2, 3, 4, 5, 34],
    });
  });

  it("falls back to the stored rect and the estimate", () => {
    const expected = estimateCaretOffsets(run(), FONTS, "Helloo", {});
    const otherRun = verdict({ ok: true, runId: "other", newRect: { x: 0, y: 0, w: 1, h: 1 }, caretOffsets: [0, 1, 2, 3, 4, 5, 6] });
    const stale = verdict({ ok: true, newRect: { x: 0, y: 0, w: 1, h: 1 }, caretOffsets: [0, 1, 2] });
    for (const v of [null, otherRun, stale, verdict({ code: "PEN_DRIFT" })]) {
      const g = editedGeometry(run(), obj, v, FONTS);
      expect(g.rect).toEqual(obj.rect);
      expect(g.caretOffsets).toEqual(expected);
      expect(g.caretOffsets).toHaveLength(7);
    }
  });
});

describe("problemMessage", () => {
  it("is empty for an ok verdict", () => {
    expect(problemMessage(verdict({ ok: true }), run())).toBe("");
  });

  it("uses the reason body for a refused line", () => {
    const v = verdict({ code: "TEXT_EDIT_REFUSED", reason: "SHARED_CONTENT" });
    expect(problemMessage(v, run())).toBe(REASON_COPY.SHARED_CONTENT.body);
    expect(problemMessage(verdict({ code: "TEXT_EDIT_REFUSED" }), refused())).toBe(REASON_COPY.SHARED_CONTENT.body);
  });

  it("names the face that is missing", () => {
    expect(problemMessage(verdict({ code: "FACE_UNAVAILABLE", face: "italic" }), run())).toBe(
      "This page has no italic version of this font.",
    );
    expect(problemMessage(verdict({ code: "FACE_UNAVAILABLE", face: "bold", chars: ["ğ"] }), run())).toBe(
      "The bold version of this font can't draw: ğ",
    );
  });

  it("picks the style sentence by field", () => {
    expect(problemMessage(verdict({ code: "STYLE_UNAVAILABLE", field: "face" }), run())).toBe(UI.style.face);
    expect(problemMessage(verdict({ code: "STYLE_UNAVAILABLE", field: "colour" }), run())).toBe(UI.style.colour);
    expect(problemMessage(verdict({ code: "STYLE_UNAVAILABLE", field: "size" }), run())).toBe(UI.style.size);
  });

  it("lists missing letters and adds the subset note when fonts are known", () => {
    const v = verdict({ code: "GLYPH_MISSING", chars: ["ğ", " "] });
    expect(problemMessage(v, run())).toBe("This document's font can't draw: ğ, space");
    expect(problemMessage(v, run(), FONTS)).toBe(
      "This document's font can't draw: ğ, space The file only includes the letters it already uses.",
    );
  });

  it("names the file for STALE and gives the fixed check sentences", () => {
    expect(problemMessage(verdict({ code: "STALE" }), run(), undefined, "a.pdf")).toBe(
      "“a.pdf” changed on disk after you started editing it. The text changes made before that can't be applied.",
    );
    expect(problemMessage(verdict({ code: "PEN_DRIFT" }), run())).toContain("would move other text");
  });
});

describe("readingNeighbour", () => {
  // A `/Rotate 90` page: Rust's display-space order (order/line) disagrees with
  // unrotated user-space positions, where the first line has the smallest y.
  function rotated(id: string, order: number, line: number, y: number): TextRun {
    return run({ id, order, line, rect: { x: 300, y, w: 12, h: 80 }, origin: { x: 300, y }, dir: { x: 0, y: 1 } });
  }
  const runs = [
    rotated("d", 3, 2, 400),
    rotated("b", 1, 0, 150),
    rotated("c", 2, 1, 300),
    rotated("a", 0, 0, 100),
  ];

  it("moves between lines in reading order, keeping the position in the line", () => {
    expect(readingNeighbour(runs, "a", "down")).toBe("c");
    expect(readingNeighbour(runs, "b", "down")).toBe("c");
    expect(readingNeighbour(runs, "c", "down")).toBe("d");
    expect(readingNeighbour(runs, "c", "up")).toBe("a");
    expect(readingNeighbour(runs, "a", "up")).toBeNull();
    expect(readingNeighbour(runs, "d", "down")).toBeNull();
  });

  it("moves along runs and wraps across lines", () => {
    expect(readingNeighbour(runs, "a", "right")).toBe("b");
    expect(readingNeighbour(runs, "b", "right")).toBe("c");
    expect(readingNeighbour(runs, "c", "left")).toBe("b");
    expect(readingNeighbour(runs, "a", "left")).toBeNull();
  });

  it("jumps to the ends and by ten lines", () => {
    expect(readingNeighbour(runs, "b", "home")).toBe("a");
    expect(readingNeighbour(runs, "a", "end")).toBe("d");
    expect(readingNeighbour(runs, "d", "end")).toBeNull();
    expect(readingNeighbour(runs, "a", "pageDown")).toBe("d");
    expect(readingNeighbour(runs, "d", "pageUp")).toBe("a");
  });

  it("starts at the first (or last) run when the current one is gone", () => {
    expect(readingNeighbour(runs, "zz", "down")).toBe("a");
    expect(readingNeighbour(runs, "zz", "end")).toBe("d");
    expect(readingNeighbour([], "a", "down")).toBeNull();
  });
});

describe("toTextEditIn / editsSignature", () => {
  const make = (id: string, runId: string, text: string, style = {}) =>
    makeSourceTextObject(id, 0, { x: 0, y: 0, w: 10, h: 10 }, {
      runId,
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: "x",
      text,
      style,
    });

  it("sends only set style fields", () => {
    const obj = make("1", "r1", "New", { sizePt: 13, fill: undefined });
    expect(toTextEditIn(obj)).toEqual({ runId: "r1", originalText: "x", text: "New", style: { sizePt: 13 } });
  });

  it("does not depend on object order or style key order", () => {
    const a = make("1", "r1", "One", { fill: "#ff0000", sizePt: 13 });
    const b = make("2", "r2", "Two");
    const a2 = make("9", "r1", "One", { sizePt: 13, fill: "#ff0000" });
    expect(editsSignature([a, b])).toBe(editsSignature([b, a2]));
    expect(editsSignature([a, b])).not.toBe(editsSignature([make("1", "r1", "One"), b]));
  });
});

describe("textStampGeometry", () => {
  const PAGE: PageGeometry = { box: { x: 0, y: 0, w: 612, h: 792 }, rotate: 0, pageIndex: 0 };
  /** Letter page turned by `/Rotate 90`: 792 × 612 as displayed. */
  const TURNED_PAGE: PageGeometry = { ...PAGE, rotate: 90 };
  const onScreen = (rect: { x: number; y: number; w: number; h: number }, g: PageGeometry) => {
    const d = displayedSize(g);
    return pdfRectToViewport(rect, makeMapping(g, d.w, d.h));
  };
  /** A refused 35° watermark: its axis-aligned box is 298 × 260 pt (live check, letter-mixed.pdf). */
  const deg = (35 * Math.PI) / 180;
  const watermark = run({
    editable: false,
    reason: "ROTATED_TEXT",
    metrics: null,
    style: null,
    text: "CONFIDENTIAL",
    rect: { x: 150, y: 250, w: 298, h: 260 },
    origin: { x: 160, y: 262 },
    dir: { x: Math.cos(deg), y: Math.sin(deg) },
  });

  it("a level line keeps its box, grown to one line and four characters (top-left kept), size clamped", () => {
    const g = textStampGeometry(run({ rect: { x: 10, y: 100, w: 200, h: 10 }, origin: { x: 10, y: 102 } }, { effectiveSize: 11.3 }), PAGE);
    expect(g.fontSize).toBe(11.5);
    expect(g.rect.x).toBeCloseTo(10, 9);
    expect(g.rect.w).toBeCloseTo(200, 9);
    expect(g.rect.h).toBeCloseTo(14.95, 9);
    expect(g.rect.y + g.rect.h).toBeCloseTo(110, 9);
    const tiny = (size: number) => textStampGeometry(run({ rect: { x: 100, y: 100, w: 1, h: 1 } }, { effectiveSize: size }), PAGE);
    expect(tiny(200).fontSize).toBe(96);
    expect(tiny(2).fontSize).toBe(6);
    expect(tiny(Number.NaN).fontSize).toBe(12);
    expect(tiny(10).rect.w).toBeCloseTo(40, 9);
    expect(textStampGeometry(watermark, PAGE).fontSize).toBe(12); // no measured size
  });

  it("a turned line gets a default one-line box at its start, not its whole axis-aligned box (live check: 298 × 260 pt)", () => {
    const g = textStampGeometry(watermark, PAGE);
    expect(g.rect.w).toBeCloseTo(120, 9); // 10 em at 12 pt
    expect(g.rect.h).toBeCloseTo(15.6, 9); // one line
    expect(g.rect.x).toBeCloseTo(160, 9); // starts where the line starts
    expect(g.rect.y + g.rect.h).toBeCloseTo(262 + 12, 9); // first baseline at the line's start
    const vertical = run({ ...watermark, dir: { x: 0, y: -1 }, reason: "VERTICAL" });
    expect(textStampGeometry(vertical, PAGE).rect.h).toBeCloseTo(15.6, 9);
    const slightlyTilted = run({ dir: { x: Math.cos(0.003), y: Math.sin(0.003) } }); // 0.17°: still level
    expect(textStampGeometry(slightlyTilted, PAGE).rect.w).toBeCloseTo(48, 9); // its own box grown to 4 em, not the 10 em default
  });

  it("stays on the page", () => {
    const g = textStampGeometry(run({ ...watermark, origin: { x: 600, y: 790 } }), PAGE);
    expect(g.rect.x + g.rect.w).toBeCloseTo(612, 9);
    expect(g.rect.y + g.rect.h).toBeCloseTo(792, 9);
  });

  it("is measured on screen on a /Rotate 90 page (one line tall there too)", () => {
    // Content counter-rotated so the line reads level on screen: +y in user space runs left to right.
    const level = run({ rect: { x: 300, y: 100, w: 12, h: 150 }, origin: { x: 303, y: 100 }, dir: { x: 0, y: 1 } });
    const box = onScreen(textStampGeometry(level, TURNED_PAGE).rect, TURNED_PAGE);
    expect(box.w).toBeCloseTo(150, 9);
    expect(box.h).toBeCloseTo(15.6, 9);
    const turned = onScreen(textStampGeometry(watermark, TURNED_PAGE).rect, TURNED_PAGE);
    expect(turned.w).toBeCloseTo(120, 9);
    expect(turned.h).toBeCloseTo(15.6, 9);
  });
});

describe("TEXT_INKS", () => {
  it("are the §A.7 ink colours in #rrggbb", () => {
    const expected: Record<string, [number, number, number]> = {
      black: [0, 0, 0],
      grey: [0.42, 0.45, 0.5],
      red: [0.78, 0.11, 0.11],
      blue: [0.11, 0.31, 0.72],
      green: [0.08, 0.45, 0.24],
      amber: [0.71, 0.33, 0.03],
    };
    expect(TEXT_INKS.map((i) => i.key)).toEqual(Object.keys(expected));
    for (const ink of TEXT_INKS) {
      const rgb = [1, 3, 5].map((i) => parseInt(ink.hex.slice(i, i + 2), 16) / 255);
      rgb.forEach((c, i) => expect(Math.abs(c - expected[ink.key][i])).toBeLessThanOrEqual(0.5 / 255));
    }
  });
});

import { describe, it, expect } from "vitest";
import { cloneObject } from "./serialize";
import {
  makeHighlightObject,
  makeInkObject,
  makeMarkupInkObject,
  makeNoteObject,
  makeStrikeoutObject,
  makeUnderlineObject,
} from "./editReducer";

describe("session markup factories", () => {
  it("stores unrotated rect, inspector author, color, and optional comment", () => {
    const rect = { x: 100, y: 200, w: 80, h: 40 };
    const note = makeNoteObject("n1", 0, rect, "Ada", "#ef4444", "sticky");
    expect(note.kind).toBe("note");
    expect(note.rect).toEqual(rect);
    expect(note.author).toBe("Ada");
    expect(note.color).toBe("#ef4444");
    expect(note.comment).toBe("sticky");

    const hl = makeHighlightObject("h1", 0, rect, "Ada", "#facc15", "review this");
    expect(hl.kind).toBe("highlight");
    expect(hl.rect).toEqual(rect);
    expect(hl.author).toBe("Ada");
    expect(hl.quads).toHaveLength(8);
    expect(JSON.parse(JSON.stringify(hl)).kind).toBe("highlight");

    const ul = makeUnderlineObject("u1", 0, rect, "Ada");
    expect(ul.kind).toBe("underline");
    expect(ul.quads).toHaveLength(8);

    const so = makeStrikeoutObject("s1", 0, rect, "Ada");
    expect(so.kind).toBe("strikeout");
    expect(so.quads).toHaveLength(8);
  });

  it("keeps Draw ink distinct from markup ink", () => {
    const draw = makeInkObject("d1", 0, [
      { x: 10, y: 10 },
      { x: 20, y: 30 },
    ]);
    expect(draw.kind).toBe("ink");

    const markup = makeMarkupInkObject(
      "m1",
      0,
      [
        [
          { x: 10, y: 10 },
          { x: 20, y: 30 },
        ],
      ],
      "Ada",
      "#111827",
    );
    expect(markup.kind).toBe("markupInk");
    expect(markup.author).toBe("Ada");
    expect(markup.strokes).toHaveLength(1);
  });
});

describe("cloneObject markup ink", () => {
  it("deep-clones markup ink strokes", () => {
    const ink = makeMarkupInkObject(
      "m1",
      0,
      [
        [
          { x: 10, y: 10 },
          { x: 20, y: 30 },
        ],
      ],
      "Ada",
    );
    const cloned = cloneObject(ink);
    if (cloned.kind !== "markupInk") throw new Error("expected markupInk");
    cloned.strokes[0][0].x = 999;
    expect(ink.strokes[0][0].x).toBe(10);
  });
});

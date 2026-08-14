import { describe, it, expect } from "vitest";
import { makeRectObject } from "./editReducer";
import { moveSelectedRects } from "./moveSelection";

describe("moveSelectedRects", () => {
  it("M1: drag on the active page does not move selected objects on other pages", () => {
    const a = makeRectObject("a", 0, { x: 10, y: 20, w: 30, h: 40 });
    const b = makeRectObject("b", 1, { x: 50, y: 60, w: 70, h: 80 });
    const bRect = { ...b.rect };

    const next = moveSelectedRects([a, b], ["a", "b"], 0, 15, -5);

    const movedA = next.find((o) => o.id === "a");
    const movedB = next.find((o) => o.id === "b");
    expect(movedA?.rect).toEqual({ x: 25, y: 15, w: 30, h: 40 });
    expect(movedB?.rect).toEqual(bRect);
    expect(movedB?.pageIndex).toBe(1);
  });
});

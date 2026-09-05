import { describe, it, expect } from "vitest";
import { makeRectObject, makeRedactObject } from "./editReducer";

describe("makeRedactObject", () => {
  it("is a distinct kind from rect with default black fill and no label", () => {
    expect(makeRedactObject).toBeTypeOf("function");
    const rect = makeRectObject("box", 0, { x: 72, y: 700, w: 120, h: 40 });
    const redact = makeRedactObject("r1", 0, { x: 72, y: 700, w: 120, h: 40 });

    expect(redact.kind).toBe("redact");
    expect(redact.kind).not.toBe("rect");
    expect(rect.kind).toBe("rect");
    expect(redact.pageIndex).toBe(0);
    expect(redact.rect).toEqual({ x: 72, y: 700, w: 120, h: 40 });
    expect(redact.fill).toBe("#000000");
    expect(redact.label).toBeUndefined();
    expect(JSON.parse(JSON.stringify(redact)).kind).toBe("redact");
  });
});

import { describe, it, expect } from "vitest";
import { makeImageObject, makeLineObject, makeTextObject } from "./editReducer";
import { isNoneFill, offsetObject, toCssHex, toExportDocument, rgbToHex } from "./serialize";

describe("toExportDocument", () => {
  it("strips previewUrl and keeps Turkish text", () => {
    const doc = {
      version: 1 as const,
      selectedIds: ["a"],
      objects: [
        makeTextObject("a", 0, { x: 10, y: 20, w: 100, h: 30 }, "GİZLİ Şğış"),
        makeImageObject("b", 0, { x: 0, y: 0, w: 40, h: 40 }, "/tmp/sign.png", "data:image/png;base64,xx"),
      ],
    };
    const out = toExportDocument(doc);
    expect(out.selectedIds).toEqual([]);
    expect(out.objects[0].kind === "text" && out.objects[0].content).toBe("GİZLİ Şğış");
    expect(out.objects[1].kind === "image" && out.objects[1].path).toBe("/tmp/sign.png");
    expect(out.objects[1].kind === "image" && out.objects[1].previewUrl).toBeUndefined();
    expect(JSON.parse(JSON.stringify(out)).objects[1].previewUrl).toBeUndefined();
  });

  it("treats none/transparent as no fill", () => {
    expect(isNoneFill("none")).toBe(true);
    expect(isNoneFill("transparent")).toBe(true);
    expect(isNoneFill("#2563eb")).toBe(false);
  });

  it("normalizes short hex and rgb to #rrggbb", () => {
    expect(toCssHex("#0af")).toBe("#00aaff");
    expect(rgbToHex(37, 99, 235)).toBe("#2563eb");
  });

  it("offsets a line's endpoints with the box", () => {
    const line = makeLineObject("l", 0, 10, 20, 40, 80);
    const next = offsetObject(line, 5, -5);
    if (next.kind !== "line") throw new Error("expected line");
    expect(next.x1).toBe(15);
    expect(next.y1).toBe(15);
    expect(next.rect.x).toBe(line.rect.x + 5);
    expect(next.id).toBe("l");
  });
});

import { describe, it, expect } from "vitest";
import { makeImageObject, makeLineObject, makeSourceTextObject, makeTextObject } from "./editReducer";
import { cloneObject, isNoneFill, offsetObject, toCssHex, toExportDocument, rgbToHex } from "./serialize";

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

describe("sourceText serialisation", () => {
  const change = () =>
    makeSourceTextObject("t", 2, { x: 72, y: 697, w: 40, h: 12 }, {
      runId: "t1:fp:0:10-20",
      sourceFingerprint: "fp",
      sourcePageIndex: 1,
      originalText: "Hello",
      text: "Hallo",
      style: { sizePt: 13, fill: "#c71c1c" },
    });

  it("deep-clones the style", () => {
    const o = change();
    const c = cloneObject(o);
    if (c.kind !== "sourceText") throw new Error("expected sourceText");
    c.style.sizePt = 99;
    c.rect.x = 0;
    expect(o.style.sizePt).toBe(13);
    expect(o.rect.x).toBe(72);
  });

  it("is never offset", () => {
    const o = change();
    const moved = offsetObject(o, 10, -10);
    expect(moved).toEqual(o);
    expect(moved).not.toBe(o);
  });

  it("exports exactly the fields Rust reads", () => {
    const o = { ...change(), objectRotate: 30, keepAspect: true, style: { sizePt: 13, face: undefined } };
    const out = toExportDocument({ version: 1, selectedIds: ["t"], objects: [o] });
    const json = JSON.parse(JSON.stringify(out.objects[0]));
    expect(Object.keys(json).sort()).toEqual(
      [
        "id",
        "kind",
        "pageIndex",
        "rect",
        "locked",
        "runId",
        "sourceFingerprint",
        "sourcePageIndex",
        "originalText",
        "text",
        "style",
      ].sort(),
    );
    expect(json).toEqual({
      id: "t",
      kind: "sourceText",
      pageIndex: 2,
      rect: { x: 72, y: 697, w: 40, h: 12 },
      locked: true,
      runId: "t1:fp:0:10-20",
      sourceFingerprint: "fp",
      sourcePageIndex: 1,
      originalText: "Hello",
      text: "Hallo",
      style: { sizePt: 13 },
    });
    expect(Object.keys((out.objects[0] as { style: object }).style)).toEqual(["sizePt"]);
  });
});

import { describe, it, expect } from "vitest";
import {
  createHistoryState,
  editReducer,
  makeRectObject,
  makeRedactObject,
} from "./editReducer";
import { toExportDocument } from "./serialize";

describe("R-NOROT redact objects do not rotate", () => {
  it("makeRedactObject has no objectRotate (shapes still may)", () => {
    const redact = makeRedactObject("r1", 0, { x: 72, y: 700, w: 120, h: 40 });
    const rect = makeRectObject("box", 0, { x: 72, y: 700, w: 120, h: 40 });

    expect(redact.kind).toBe("redact");
    expect(redact.objectRotate).toBeUndefined();
    expect(Object.prototype.hasOwnProperty.call(redact, "objectRotate")).toBe(false);
    expect(JSON.parse(JSON.stringify(redact)).objectRotate).toBeUndefined();

    // Contrast: a shape may carry objectRotate; redaction must not.
    const rotatedRect = { ...rect, objectRotate: 15 };
    expect(rotatedRect.objectRotate).toBe(15);
  });

  it("UPDATE objectRotate on a redact is ignored and export omits it", () => {
    let s = createHistoryState();
    s = editReducer(s, {
      type: "ADD",
      object: makeRedactObject("r1", 0, { x: 72, y: 700, w: 120, h: 40 }),
    });
    s = editReducer(s, { type: "UPDATE", id: "r1", patch: { objectRotate: 45 } });

    const after = s.present.objects[0];
    expect(after.kind).toBe("redact");
    expect(after.objectRotate).toBeUndefined();

    const exported = toExportDocument(s.present);
    expect(exported.objects[0].kind).toBe("redact");
    expect(exported.objects[0].objectRotate).toBeUndefined();
    expect(JSON.parse(JSON.stringify(exported)).objects[0].objectRotate).toBeUndefined();
  });
});

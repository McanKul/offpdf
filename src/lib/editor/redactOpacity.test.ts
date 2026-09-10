import { describe, it, expect } from "vitest";
import {
  createHistoryState,
  editReducer,
  makeRectObject,
  makeRedactObject,
} from "./editReducer";
import type { EditObject } from "./types";
import { toExportDocument } from "./serialize";

describe("R-OPACITY-UPDATE redact objects do not take opacity", () => {
  it("makeRedactObject has no opacity (shapes still may)", () => {
    const redact = makeRedactObject("r1", 0, { x: 72, y: 700, w: 120, h: 40 });
    const rect = makeRectObject("box", 0, { x: 72, y: 700, w: 120, h: 40 });

    expect(redact.kind).toBe("redact");
    expect((redact as { opacity?: number }).opacity).toBeUndefined();
    expect(Object.prototype.hasOwnProperty.call(redact, "opacity")).toBe(false);
    expect(JSON.parse(JSON.stringify(redact)).opacity).toBeUndefined();

    // Contrast: a shape may carry opacity; redaction must not.
    const fadedRect = { ...rect, opacity: 0.4 };
    expect(fadedRect.opacity).toBe(0.4);
  });

  it("UPDATE opacity on a redact is ignored and export omits it", () => {
    let s = createHistoryState();
    s = editReducer(s, {
      type: "ADD",
      object: makeRedactObject("r1", 0, { x: 72, y: 700, w: 120, h: 40 }),
    });
    s = editReducer(s, {
      type: "UPDATE",
      id: "r1",
      patch: { opacity: 0.4 } as Partial<EditObject>,
    });

    const after = s.present.objects[0];
    expect(after.kind).toBe("redact");
    expect((after as { opacity?: number }).opacity).toBeUndefined();
    expect(Object.prototype.hasOwnProperty.call(after, "opacity")).toBe(false);

    const exported = toExportDocument(s.present);
    expect(exported.objects[0].kind).toBe("redact");
    expect((exported.objects[0] as { opacity?: number }).opacity).toBeUndefined();
    expect(JSON.parse(JSON.stringify(exported)).objects[0].opacity).toBeUndefined();
    expect(Object.prototype.hasOwnProperty.call(exported.objects[0], "opacity")).toBe(false);
  });
});

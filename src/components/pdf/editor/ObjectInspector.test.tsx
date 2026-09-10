import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { makeRectObject, type EditObject } from "@/lib/editor";
import { ObjectInspector } from "./ObjectInspector";

function redactObject(): EditObject {
  return {
    id: "r1",
    kind: "redact",
    pageIndex: 0,
    rect: { x: 72, y: 700, w: 120, h: 40 },
    fill: "#000000",
  } as unknown as EditObject;
}

function renderInspector(obj: EditObject): string {
  return renderToStaticMarkup(
    <ObjectInspector obj={obj} layerIndex={1} layerCount={1} onChange={() => {}} />,
  );
}

describe("R-OPACITY-UI redact inspector hides Opacity", () => {
  it("redact markup has no Opacity; rectangle still has it", () => {
    const redact = renderInspector(redactObject());
    const rect = renderInspector(makeRectObject("box", 0, { x: 72, y: 700, w: 120, h: 40 }));

    expect(redact).not.toContain('aria-label="Opacity"');
    expect(redact).not.toContain("Opacity");
    expect(rect).toContain('aria-label="Opacity"');
  });
});

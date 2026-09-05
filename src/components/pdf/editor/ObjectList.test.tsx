import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { makeRectObject, type EditObject } from "@/lib/editor";
import { TOOLS } from "@/lib/tools";
import { ObjectList } from "./ObjectList";

function redactObject(): EditObject {
  return {
    id: "r1",
    kind: "redact",
    pageIndex: 0,
    rect: { x: 72, y: 700, w: 120, h: 40 },
    fill: "#000000",
  } as unknown as EditObject;
}

describe("ObjectList redaction vs rectangle", () => {
  it("labels a redact object Redaction, not Rectangle", () => {
    const rect = makeRectObject("box", 0, { x: 10, y: 20, w: 100, h: 50 });
    const redact = redactObject();
    const mixed = renderToStaticMarkup(
      <ObjectList
        objects={[rect, redact]}
        selectedIds={[]}
        onSelect={() => {}}
        onDelete={() => {}}
      />,
    );
    const onlyRedact = renderToStaticMarkup(
      <ObjectList objects={[redact]} selectedIds={[]} onSelect={() => {}} onDelete={() => {}} />,
    );

    expect(mixed).toContain("Rectangle");
    expect(onlyRedact).toContain("Redaction");
    expect(onlyRedact).not.toContain("Rectangle");
  });

  it("does not register redaction as a home-grid tool", () => {
    expect(TOOLS.some((t) => t.path.includes("redact"))).toBe(false);
    expect(TOOLS.some((t) => t.id === "editPdf")).toBe(true);
  });
});

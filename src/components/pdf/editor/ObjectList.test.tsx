import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { makeRectObject, makeSourceTextObject, type EditObject } from "@/lib/editor";
import { UI, fillCopy } from "@/lib/editor/sourceTextCopy";
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

describe("ObjectList text changes", () => {
  const change = (text: string, pageIndex = 2): EditObject =>
    makeSourceTextObject(`st-${text}`, pageIndex, { x: 72, y: 700, w: 40, h: 12 }, {
      runId: "t1:fp:2:10-20",
      sourceFingerprint: "fp",
      sourcePageIndex: 2,
      originalText: "Invoice 2026",
      text,
      style: {},
    });

  it("labels a text change `Edited text: “…” · pN`, never Rectangle or a layer", () => {
    const markup = renderToStaticMarkup(
      <ObjectList objects={[change("Invoice 2027")]} selectedIds={[]} onSelect={() => {}} onDelete={() => {}} />,
    );
    expect(markup).toContain(fillCopy(UI.list.label, { new: "Invoice 2027", page: 3 }));
    expect(markup).toContain("Edited text: “Invoice 2027” · p3");
    expect(markup).not.toContain("Rectangle");
    expect(markup).not.toContain("1/1");
  });

  it("cuts long new text at 28 characters", () => {
    const long = "A".repeat(40);
    const markup = renderToStaticMarkup(
      <ObjectList objects={[change(long)]} selectedIds={[]} onSelect={() => {}} onDelete={() => {}} />,
    );
    expect(markup).toContain(`Edited text: “${"A".repeat(28)}…” · p3`);
  });

  it("does not count text changes as layers of the other objects", () => {
    const rect = makeRectObject("box", 2, { x: 10, y: 20, w: 100, h: 50 });
    const markup = renderToStaticMarkup(
      <ObjectList objects={[change("x"), rect]} selectedIds={[]} onSelect={() => {}} onDelete={() => {}} />,
    );
    expect(markup).toContain("Rectangle · p3 · 1/1");
  });
});

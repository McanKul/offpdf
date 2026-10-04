import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { makeRectObject, makeSourceTextObject, type EditObject } from "@/lib/editor";
import { UI } from "@/lib/editor/sourceTextCopy";
import type { TextFont, TextRun } from "@/lib/types";
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

describe("ObjectInspector text change branch (§D.6.4)", () => {
  const font: TextFont = {
    key: "f1",
    displayName: "Calibri",
    familyHint: "sans",
    embedded: true,
    subset: true,
    alphabet: " 0267Iceinov",
    widths: [226, 507, 507, 507, 507, 252, 423, 498, 229, 525, 527, 498],
    wordSpace: true,
  };
  const run: TextRun = {
    id: "t1:fp:0:10-20",
    order: 0,
    line: 0,
    text: "Invoice 2026",
    rect: { x: 72, y: 697, w: 60, h: 12 },
    origin: { x: 72, y: 700 },
    dir: { x: 1, y: 0 },
    ascent: 9,
    descent: 3,
    caretOffsets: [],
    editable: true,
    reason: null,
    metrics: {
      surface: ["f1"],
      tfSize: 12,
      effectiveSize: 12,
      charSpacing: 0,
      wordSpacing: 0,
      hScale: 1,
      textToUser: 1,
      letterSpacingPt: 0,
      spaceMode: "glyph",
      kernSpace: -250,
      originalWidth: 60,
      visibleExtent: 400,
      nextObstacle: null,
    },
    style: {
      fill: "#000000",
      sizeChangeable: true,
      colourChangeable: true,
      face: "regular",
      faces: {
        regular: { available: true, surface: ["f1"] },
        bold: { available: true, surface: ["f1"] },
        italic: { available: false, surface: [] },
        boldItalic: { available: false, surface: [] },
      },
    },
    substituted: false,
  };
  const change = makeSourceTextObject("st1", 0, run.rect, {
    runId: run.id,
    sourceFingerprint: "fp",
    sourcePageIndex: 0,
    originalText: "Invoice 2026",
    text: "Invoice 2027",
    style: { face: "bold", fill: "#c71c1c" },
  });

  function render(r: TextRun | null = run): string {
    return renderToStaticMarkup(
      <ObjectInspector
        obj={change}
        layerIndex={1}
        layerCount={1}
        onChange={() => {}}
        onReorder={() => {}}
        sourceText={{ run: r, fonts: new Map([["f1", font]]), onEditLine: () => {}, onRestore: () => {} }}
      />,
    );
  }

  it("shows Text change with Original, New, Font, Letters, Style and the note", () => {
    const markup = render();
    expect(markup).toContain(UI.inspector.header);
    expect(markup).toContain(`<dt>${UI.inspector.original}</dt><dd>Invoice 2026</dd>`);
    expect(markup).toContain(`<dt>${UI.inspector.new}</dt><dd>Invoice 2027</dd>`);
    expect(markup).toContain("Calibri · embedded subset");
    expect(markup).toContain(`<dt>${UI.bar.letters}</dt>`);
    expect(markup).toContain("space 0267Iceinov");
    expect(markup).toContain("12 pt · Bold · Red");
    expect(markup).toContain(UI.inspector.note);
    expect(markup).toContain(UI.inspector.editLine);
    expect(markup).toContain(UI.inspector.restore);
  });

  it("hides W/H, Rotation, Opacity and Layer", () => {
    const markup = render();
    for (const hidden of ['aria-label="W"', 'aria-label="H"', "Rotation", "Opacity", "Layer", "Send to back"]) {
      expect(markup).not.toContain(hidden);
    }
  });

  it("still shows the change before the page's lines are read (no font row yet)", () => {
    const markup = render(null);
    expect(markup).toContain("Invoice 2027");
    expect(markup).not.toContain(`<dt>${UI.inspector.font}</dt>`);
  });

  it("names a removed line with the removal note", () => {
    const removed = { ...change, text: "" };
    const markup = renderToStaticMarkup(
      <ObjectInspector obj={removed} layerIndex={1} layerCount={1} onChange={() => {}}
        sourceText={{ run, fonts: new Map([["f1", font]]), onEditLine: () => {}, onRestore: () => {} }} />,
    );
    expect(markup).toContain(UI.removal);
  });
});

// @vitest-environment happy-dom
import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fontsByKey, makeSourceTextObject, type SourceTextObject } from "@/lib/editor";
import { UI, fillCopy } from "@/lib/editor/sourceTextCopy";
import type { TextFont, TextRun } from "@/lib/types";
import type { PageLayout } from "../PageSurface";
import { SourceTextLayer, type SourceTextLayerProps } from "./SourceTextLayer";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const FONT: TextFont = {
  key: "f1",
  displayName: "Arial",
  familyHint: "sans",
  embedded: true,
  subset: false,
  alphabet: " 0123456789ITacdeilnostuv",
  widths: Array.from({ length: 25 }, () => 500),
  wordSpace: true,
};

/** Letter page, 1 CSS px per point; CSS y = 792 − PDF y. */
const LAYOUT: PageLayout = { cssWidth: 612, cssHeight: 792, geometry: { box: { x: 0, y: 0, w: 612, h: 792 }, rotate: 0, pageIndex: 0 } };

function run(id: string, order: number, line: number, text: string, x: number, y: number, extra: Partial<TextRun> = {}): TextRun {
  const w = text.length * 6;
  return {
    id,
    order,
    line,
    text,
    rect: { x, y: y - 3, w, h: 12 },
    origin: { x, y },
    dir: { x: 1, y: 0 },
    ascent: 9,
    descent: 3,
    caretOffsets: Array.from({ length: text.length + 1 }, (_, i) => i * 6),
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
      originalWidth: w,
      visibleExtent: 500,
      nextObstacle: null,
    },
    style: {
      fill: "#000000",
      sizeChangeable: true,
      colourChangeable: true,
      face: "regular",
      faces: {
        regular: { available: true, surface: ["f1"] },
        bold: { available: false, surface: [] },
        italic: { available: false, surface: [] },
        boldItalic: { available: false, surface: [] },
      },
    },
    substituted: false,
    ...extra,
  };
}

const INVOICE = run("r-invoice", 0, 0, "Invoice 2026", 72, 700);
const TOTAL = run("r-total", 1, 0, "Total", 300, 700);
const SHARED = run("r-shared", 2, 1, "Letterhead", 72, 650, { editable: false, reason: "SHARED_CONTENT", metrics: null, style: null });
const LOW = run("r-low", 3, 2, "Footer", 72, 60);

let root: Root | null = null;
let host: HTMLElement;
let stage: HTMLElement;

type Spies = Pick<SourceTextLayerProps, "onEdit" | "onExplain" | "onRestore" | "onLeave">;

function mount(runs: TextRun[], extra: Partial<SourceTextLayerProps> = {}, stageHeight = 792): Spies {
  const spies: Spies = { onEdit: vi.fn(), onExplain: vi.fn(), onRestore: vi.fn(), onLeave: vi.fn() };
  stage = document.createElement("div");
  stage.getBoundingClientRect = () => ({ left: 0, top: 0, right: 612, bottom: stageHeight, width: 612, height: stageHeight, x: 0, y: 0, toJSON: () => ({}) });
  host = document.createElement("div");
  stage.appendChild(host);
  document.body.appendChild(stage);
  function Harness() {
    const [focus, setFocus] = useState<{ id: string | null; tick: number }>({ id: extra.focusedId ?? null, tick: 0 });
    return createElement(SourceTextLayer, {
      runs,
      objects: [],
      fonts: fontsByKey([FONT]),
      layout: LAYOUT,
      pageNumber: 3,
      preview: null,
      duplicate: false,
      editingId: null,
      popoverId: null,
      stage,
      inert: false,
      ...extra,
      ...spies,
      focusedId: focus.id,
      focusTick: focus.tick,
      onFocusedChange: (id: string, move: boolean) => setFocus((f) => ({ id, tick: move ? f.tick + 1 : f.tick })),
    });
  }
  root = createRoot(host);
  act(() => root!.render(createElement(Harness)));
  return spies;
}

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  stage.remove();
});

const runButtons = () => [...host.querySelectorAll<HTMLButtonElement>("button.st-run")];
const buttonFor = (r: TextRun) => runButtons().find((b) => b.getAttribute("aria-label")?.includes(r.text))!;
const layer = () => host.querySelector<HTMLElement>(".st-layer")!;

function key(el: Element, k: string): KeyboardEvent {
  const ev = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true });
  act(() => {
    el.dispatchEvent(ev);
  });
  return ev;
}

describe("SourceTextLayer", () => {
  it("is a labelled group of buttons in reading order with one Tab stop", () => {
    mount([SHARED, TOTAL, INVOICE]);
    expect(layer().getAttribute("role")).toBe("group");
    expect(layer().getAttribute("aria-label")).toBe(fillCopy(UI.layer.label, { n: 3 }));
    expect(layer().hasAttribute("data-source-text")).toBe(true);
    expect(runButtons().map((b) => b.getAttribute("aria-label"))).toEqual([
      "Edit text: Invoice 2026",
      "Edit text: Total",
      "Letterhead",
    ]);
    expect(runButtons().filter((b) => b.tabIndex === 0)).toEqual([buttonFor(INVOICE)]);
  });

  it("arrows move focus in reading order (→ along the line, ↓ to the next line)", () => {
    mount([INVOICE, TOTAL, SHARED]);
    act(() => buttonFor(INVOICE).focus());
    key(buttonFor(INVOICE), "ArrowRight");
    expect(document.activeElement).toBe(buttonFor(TOTAL));
    key(buttonFor(TOTAL), "ArrowDown");
    expect(document.activeElement).toBe(buttonFor(SHARED));
    key(buttonFor(SHARED), "Home");
    expect(document.activeElement).toBe(buttonFor(INVOICE));
    expect(runButtons().filter((b) => b.tabIndex === 0)).toEqual([buttonFor(INVOICE)]);
  });

  it("a refused line is aria-disabled and described as Can't be changed: {short}", () => {
    mount([INVOICE, SHARED]);
    const b = buttonFor(SHARED);
    expect(b.getAttribute("aria-disabled")).toBe("true");
    const described = document.getElementById(b.getAttribute("aria-describedby")!);
    expect(described?.textContent).toBe("Can't be changed: shared with other pages");
  });

  it("Enter (and F2) on an editable line opens the editor with all text selected", () => {
    const spies = mount([INVOICE, SHARED]);
    key(buttonFor(INVOICE), "Enter");
    expect(spies.onEdit).toHaveBeenCalledWith(INVOICE, "all");
    key(buttonFor(INVOICE), "F2");
    expect(spies.onEdit).toHaveBeenCalledTimes(2);
  });

  it("Enter on a refused line opens the reason popover", () => {
    const spies = mount([INVOICE, SHARED]);
    key(buttonFor(SHARED), "Enter");
    expect(spies.onExplain).toHaveBeenCalledWith(SHARED);
    expect(spies.onEdit).not.toHaveBeenCalled();
  });

  it("Delete restores the original of an edited line; Esc leaves the layer", () => {
    const change = makeSourceTextObject("c1", 0, INVOICE.rect, {
      runId: INVOICE.id,
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: INVOICE.text,
      text: "Invoice 2027",
      style: {},
    });
    const spies = mount([INVOICE, TOTAL], { objects: [change] });
    const edited = runButtons()[0];
    expect(edited.getAttribute("aria-label")).toBe("Edited text: Invoice 2027. Original: Invoice 2026");
    key(edited, "Delete");
    expect(spies.onRestore).toHaveBeenCalledWith(change);
    key(buttonFor(TOTAL), "Backspace");
    expect(spies.onRestore).toHaveBeenCalledTimes(1);
    key(buttonFor(TOTAL), "Escape");
    expect(spies.onLeave).toHaveBeenCalledTimes(1);
  });

  it("an edited line whose new text ends a sentence is labelled with one full stop (live check: “days..”)", () => {
    const change = makeSourceTextObject("c1", 0, INVOICE.rect, {
      runId: INVOICE.id,
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: INVOICE.text,
      text: "Paid within 30 days.",
      style: {},
    });
    mount([INVOICE], { objects: [change] });
    const label = runButtons()[0].getAttribute("aria-label");
    expect(label).toBe("Edited text: Paid within 30 days. Original: Invoice 2026");
    expect(label).not.toContain("..");
  });

  it("hides a refused line's tooltip while its reason dialog is open (live check: tooltip under the dialog)", () => {
    vi.useFakeTimers();
    try {
      const props = (popoverId: string | null) =>
        createElement(SourceTextLayer, {
          runs: [SHARED], objects: [], fonts: fontsByKey([FONT]), layout: LAYOUT, pageNumber: 1, preview: null, duplicate: false,
          focusedId: null, focusTick: 0, editingId: null, popoverId, stage: null, inert: false,
          onFocusedChange: () => {}, onEdit: vi.fn(), onExplain: vi.fn(), onRestore: vi.fn(), onLeave: vi.fn(),
        });
      stage = document.createElement("div");
      host = document.createElement("div");
      stage.appendChild(host);
      document.body.appendChild(stage);
      root = createRoot(host);
      act(() => root!.render(props(null)));
      // Hover the dotted line (CSS y = 792 − 650) until its tooltip shows.
      act(() => {
        layer().dispatchEvent(new PointerEvent("pointermove", { clientX: 80, clientY: 792 - 650, bubbles: true }));
      });
      act(() => vi.advanceTimersByTime(450));
      const tip = () => host.querySelector('[role="tooltip"]');
      expect(tip()?.textContent).toBe("Can't be changed: shared with other pages");
      // A click opens the reason dialog for that line; the pointer is still over it.
      act(() => root!.render(props(SHARED.id)));
      act(() => vi.advanceTimersByTime(450));
      expect(tip()).toBeNull();
      // Closed again (still hovering): the tooltip may come back.
      act(() => root!.render(props(null)));
      expect(tip()).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("renders only lines near the visible stage, but keeps the focused line rendered", () => {
    mount([INVOICE, LOW], {}, 150);
    expect(runButtons().map((b) => b.getAttribute("aria-label"))).toEqual(["Edit text: Invoice 2026"]);
    act(() => root!.unmount());
    stage.remove();
    mount([INVOICE, LOW], { focusedId: LOW.id }, 150);
    expect(runButtons().map((b) => b.getAttribute("aria-label"))).toEqual(["Edit text: Invoice 2026", "Edit text: Footer"]);
  });

  it("a click on the extended part of an edited line hits that line, caret within the new text", () => {
    const newRect = { x: 72, y: 697, w: 120, h: 12 };
    const change: SourceTextObject = makeSourceTextObject("c1", 0, newRect, {
      runId: INVOICE.id,
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: INVOICE.text,
      text: "Invoice 2026 (paid in full)",
      style: {},
    });
    const spies = mount([INVOICE, TOTAL], { objects: [change] });
    // Original box ends at x = 72 + 72; the click is at x = 180, inside the new box only.
    act(() => {
      layer().dispatchEvent(new MouseEvent("click", { clientX: 180, clientY: 792 - 700 + 1, bubbles: true }));
    });
    expect(spies.onEdit).toHaveBeenCalledTimes(1);
    const [hit, caret] = (spies.onEdit as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(hit).toBe(INVOICE);
    expect(caret).toBeGreaterThan(INVOICE.text.length);
    expect(caret).toBeLessThanOrEqual(Array.from(change.text).length);
  });

  it("a click on an unedited line puts the caret at the nearest boundary; a refused one explains", () => {
    const spies = mount([INVOICE, SHARED]);
    act(() => {
      layer().dispatchEvent(new MouseEvent("click", { clientX: 72 + 13, clientY: 92, bubbles: true }));
    });
    expect(spies.onEdit).toHaveBeenCalledWith(INVOICE, 2);
    act(() => {
      layer().dispatchEvent(new MouseEvent("click", { clientX: 80, clientY: 792 - 650, bubbles: true }));
    });
    expect(spies.onExplain).toHaveBeenCalledWith(SHARED);
  });

  it("a page listed twice is shown refused, described by banner.duplicatePage, and never edited", () => {
    const spies = mount([INVOICE], { duplicate: true });
    const b = runButtons()[0];
    expect(b.getAttribute("aria-label")).toBe("Invoice 2026");
    expect(b.getAttribute("aria-disabled")).toBe("true");
    expect(b.className).toContain("is-refused");
    expect(document.getElementById(b.getAttribute("aria-describedby")!)?.textContent).toContain(UI.banner.duplicatePage);
    key(b, "Enter");
    act(() => {
      layer().dispatchEvent(new MouseEvent("click", { clientX: 80, clientY: 92, bubbles: true }));
    });
    expect(spies.onEdit).not.toHaveBeenCalled();
    expect(spies.onExplain).not.toHaveBeenCalled();
  });

  it("Space on a focused line activates it and never reaches the page's Space-to-pan", () => {
    const spies = mount([INVOICE]);
    const pan = vi.fn();
    window.addEventListener("keydown", pan);
    act(() => buttonFor(INVOICE).focus());
    const ev = key(buttonFor(INVOICE), " ");
    window.removeEventListener("keydown", pan);
    expect(spies.onEdit).toHaveBeenCalledWith(INVOICE, "all");
    expect(ev.defaultPrevented).toBe(true);
    expect(pan).not.toHaveBeenCalled();
    expect(buttonFor(INVOICE).closest("[data-source-text]")).not.toBeNull();
  });
});

// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fontsByKey } from "@/lib/editor";
import { UI, charList } from "@/lib/editor/sourceTextCopy";
import type { SourceTextStyle, TextFont, TextRun } from "@/lib/types";
import { TextFormatBar, steppedSize, toggledFace, type TextFormatBarProps } from "./TextFormatBar";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const SUBSET: TextFont = {
  key: "f1",
  displayName: "Calibri",
  familyHint: "sans",
  embedded: true,
  subset: true,
  alphabet: " Haelo",
  widths: [226, 631, 479, 229, 229, 527],
  wordSpace: true,
};
const BOLD: TextFont = { ...SUBSET, key: "fb", alphabet: " Hael" };
const FULL: TextFont = { ...SUBSET, key: "f9", subset: false };

function run(style: Partial<NonNullable<TextRun["style"]>> = {}, surface = ["f1"]): TextRun {
  return {
    id: "r1",
    order: 0,
    line: 0,
    text: "Hello",
    rect: { x: 72, y: 697, w: 30, h: 12 },
    origin: { x: 72, y: 700 },
    dir: { x: 1, y: 0 },
    ascent: 9,
    descent: 3,
    caretOffsets: [0, 6, 12, 18, 24, 30],
    editable: true,
    reason: null,
    metrics: {
      surface,
      tfSize: 12,
      effectiveSize: 12,
      charSpacing: 0,
      wordSpacing: 0,
      hScale: 1,
      textToUser: 1,
      letterSpacingPt: 0,
      spaceMode: "glyph",
      kernSpace: -250,
      originalWidth: 30,
      visibleExtent: 400,
      nextObstacle: null,
    },
    style: {
      fill: "#000000",
      sizeChangeable: true,
      colourChangeable: true,
      face: "regular",
      faces: {
        regular: { available: true, surface },
        bold: { available: true, surface: ["fb"] },
        italic: { available: false, surface: [] },
        boldItalic: { available: false, surface: [] },
      },
      ...style,
    },
    substituted: false,
  };
}

let root: Root | null = null;
let host: HTMLElement;

function mount(r: TextRun, style: SourceTextStyle = {}, extra: Partial<TextFormatBarProps> = {}) {
  const props: TextFormatBarProps = {
    run: r,
    fonts: fontsByKey([SUBSET, BOLD, FULL]),
    style,
    onStyle: vi.fn(),
    onNotice: vi.fn(),
    canRestore: false,
    onRestore: vi.fn(),
    onCancel: vi.fn(),
    onDone: vi.fn(),
    onBackToInput: vi.fn(),
    placement: "above",
    ...extra,
  };
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root!.render(createElement(TextFormatBar, props)));
  return props;
}

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  host.remove();
});

const button = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
const key = (el: Element, k: string, init: KeyboardEventInit = {}) =>
  act(() => {
    el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
  });

describe("TextFormatBar", () => {
  it("is a toolbar named Text format", () => {
    mount(run());
    const bar = host.querySelector('[role="toolbar"]')!;
    expect(bar.getAttribute("aria-label")).toBe(UI.bar.label);
  });

  it("B8: an unavailable italic stays focusable and writes the exact italic sentence", () => {
    const props = mount(run());
    const italic = button(UI.bar.italic);
    expect(italic.getAttribute("aria-disabled")).toBe("true");
    expect(italic.getAttribute("title")).toBe("This page has no italic version of this font.");
    expect(italic.disabled).toBe(false);
    act(() => italic.click());
    expect(props.onNotice).toHaveBeenCalledWith("This page has no italic version of this font.");
    expect(props.onStyle).not.toHaveBeenCalled();
  });

  it("B9: a font set through ExtGState makes B and I write style.face, and size write style.size", () => {
    const props = mount(run({ sizeChangeable: false }));
    act(() => button(UI.bar.bold).click());
    act(() => button(UI.bar.italic).click());
    expect(props.onNotice).toHaveBeenNthCalledWith(1, UI.style.face);
    expect(props.onNotice).toHaveBeenNthCalledWith(2, UI.style.face);
    act(() => button(UI.bar.larger).click());
    expect(props.onNotice).toHaveBeenLastCalledWith(UI.style.size);
    expect(host.querySelector(`input[aria-label="${UI.bar.fontSize}"]`)).toBeNull();
  });

  it("an available bold switches the face; aria-pressed follows the face", () => {
    const props = mount(run());
    expect(button(UI.bar.bold).getAttribute("aria-pressed")).toBe("false");
    act(() => button(UI.bar.bold).click());
    expect(props.onStyle).toHaveBeenCalledWith({ face: "bold" });
    act(() => root!.unmount());
    host.remove();
    mount(run({ face: "bold" }));
    expect(button(UI.bar.bold).getAttribute("aria-pressed")).toBe("true");
  });

  it("colour unavailable (outline text) writes style.colour", () => {
    const props = mount(run({ colourChangeable: false }));
    act(() => button(UI.bar.textColour).click());
    expect(props.onNotice).toHaveBeenCalledWith(UI.style.colour);
  });

  it("every button keeps the input's focus on mouse down", () => {
    mount(run(), {}, { canRestore: true });
    const all = [...host.querySelectorAll<HTMLButtonElement>("button")];
    expect(all.length).toBeGreaterThan(8);
    for (const b of all) {
      const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
      act(() => {
        b.dispatchEvent(down);
      });
      expect(down.defaultPrevented, b.getAttribute("aria-label") ?? b.textContent ?? "").toBe(true);
    }
  });

  it("roving tabindex: one tab stop, ←/→ move focus, Esc and Shift+Tab go back to the input", () => {
    const props = mount(run());
    const items = () => [...host.querySelectorAll<HTMLElement>("[data-roving]")];
    expect(items().filter((el) => el.tabIndex === 0)).toHaveLength(1);
    const bold = button(UI.bar.bold);
    act(() => bold.focus());
    key(bold, "ArrowRight");
    expect(document.activeElement).toBe(button(UI.bar.italic));
    key(document.activeElement!, "ArrowLeft");
    expect(document.activeElement).toBe(bold);
    expect(items().filter((el) => el.tabIndex === 0)).toEqual([bold]);
    key(bold, "Escape");
    key(bold, "Tab", { shiftKey: true });
    expect(props.onBackToInput).toHaveBeenCalledTimes(2);
  });

  it("Letters: only for an embedded subset, listing the current face's alphabet with space named", () => {
    mount(run());
    const letters = [...host.querySelectorAll("button")].find((b) => b.textContent === UI.bar.letters)!;
    expect(letters).toBeDefined();
    act(() => letters.click());
    const pop = host.querySelector(".st-bar__letters")!;
    expect(pop.textContent).toContain(UI.bar.lettersTitle);
    expect(pop.textContent).toContain(charList(Array.from(" Haelo")));
    expect(pop.textContent).toContain("space, H, a, e, l, o");

    act(() => root!.unmount());
    host.remove();
    mount(run(), { face: "bold" }); // the draft's face decides which letters can be typed
    const boldLetters = [...host.querySelectorAll("button")].find((b) => b.textContent === UI.bar.letters)!;
    act(() => boldLetters.click());
    expect(host.querySelector(".st-bar__letters")!.textContent).toContain("space, H, a, e, l");
    expect(host.querySelector(".st-bar__letters")!.textContent).not.toContain("l, o");

    act(() => root!.unmount());
    host.remove();
    mount(run({}, ["f9"]));
    expect([...host.querySelectorAll("button")].some((b) => b.textContent === UI.bar.letters)).toBe(false);
  });

  describe("a popover never covers the line being edited (live check: Letters over the inline box)", () => {
    // The real layout, measured: the stage, the bar docked above the line, the line (chip) and its
    // message row under it. happy-dom has no layout, so the boxes are given here.
    type Box = { top: number; bottom: number };
    const boxes = new Map<string, Box>();
    let restore: Array<() => void> = [];
    afterEach(() => {
      restore.forEach((r) => r());
      restore = [];
      boxes.clear();
    });

    function mountDocked(barTop: number): HTMLElement {
      boxes.set("pdf-editor__stage", { top: 0, bottom: 600 });
      boxes.set("st-bar", { top: barTop, bottom: barTop + 36 });
      boxes.set("st-chip", { top: barTop + 42, bottom: barTop + 60 });
      boxes.set("st-msg", { top: barTop + 64, bottom: barTop + 82 });
      const rect = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
        const hit = [...boxes.entries()].find(([cls]) => this.classList.contains(cls))?.[1] ?? { top: 0, bottom: 0 };
        return { x: 0, y: hit.top, left: 0, right: 400, top: hit.top, bottom: hit.bottom, width: 400, height: hit.bottom - hit.top, toJSON: () => ({}) };
      });
      const height = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
        return this.classList.contains("st-bar__pop") ? 80 : 0;
      });
      restore = [() => rect.mockRestore(), () => height.mockRestore()];
      const stage = document.createElement("div");
      stage.className = "pdf-editor__stage";
      const editor = document.createElement("div");
      editor.className = "st-editor";
      const dock = document.createElement("div");
      dock.className = "st-dock st-dock--above";
      const chip = document.createElement("div");
      chip.className = "st-chip";
      const msg = document.createElement("div");
      msg.className = "st-msg";
      editor.append(dock, chip, msg);
      stage.append(editor);
      host = stage;
      document.body.appendChild(stage);
      root = createRoot(dock);
      const props: TextFormatBarProps = {
        run: run(), fonts: fontsByKey([SUBSET, BOLD, FULL]), style: {}, onStyle: vi.fn(), onNotice: vi.fn(), canRestore: false,
        onRestore: vi.fn(), onCancel: vi.fn(), onDone: vi.fn(), onBackToInput: vi.fn(), placement: "above",
      };
      act(() => root!.render(createElement(TextFormatBar, props)));
      const letters = [...stage.querySelectorAll("button")].find((b) => b.textContent === UI.bar.letters)!;
      act(() => letters.click());
      return stage.querySelector<HTMLElement>(".st-bar__letters")!;
    }

    it("without room above the bar, it opens below the line and its message row, not over them", () => {
      const pop = mountDocked(40); // 40 px above the bar: an 80 px popover can't go up
      expect(pop.classList.contains("is-up")).toBe(false);
      // Bar bottom 76; the message row reaches 122: 46 px to clear, plus the 4 px gap.
      expect(pop.style.top).toBe("calc(100% + 50px)");
    });

    it("with room above the bar, it opens upward, away from the line", () => {
      const pop = mountDocked(300);
      expect(pop.classList.contains("is-up")).toBe(true);
      expect(pop.style.top).toBe("");
    });
  });

  it("colour popover: Original colour drops the fill, an ink sets it", () => {
    const props = mount(run(), { fill: "#c71c1c" });
    act(() => button(UI.bar.textColour).click());
    const original = [...host.querySelectorAll("button")].find((b) => b.textContent === UI.bar.originalColour)!;
    act(() => original.click());
    expect(props.onStyle).toHaveBeenLastCalledWith({});
    act(() => button(UI.bar.blue).click());
    expect(props.onStyle).toHaveBeenLastCalledWith({ fill: "#1c4fb8" });
  });

  it("Restore original appears only for a committed change; Cancel and Done call back", () => {
    const props = mount(run(), {}, { canRestore: true });
    const byText = (t: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.includes(t))!;
    act(() => byText(UI.bar.restore).click());
    act(() => byText(UI.bar.cancel).click());
    act(() => byText(UI.bar.done).click());
    expect(props.onRestore).toHaveBeenCalledTimes(1);
    expect(props.onCancel).toHaveBeenCalledTimes(1);
    expect(props.onDone).toHaveBeenCalledTimes(1);
    act(() => root!.unmount());
    host.remove();
    mount(run());
    expect([...host.querySelectorAll("button")].some((b) => b.textContent === UI.bar.restore)).toBe(false);
  });
});

describe("format helpers (shared with ⌘B / ⌘I / ⌘⇧. / ⌘⇧,)", () => {
  it("steps the effective size by ±0.5 within 4–144", () => {
    expect(steppedSize(run(), {}, 0.5)).toEqual({ style: { sizePt: 12.5 } });
    expect(steppedSize(run(), { sizePt: 4.2 }, -0.5)).toEqual({ style: { sizePt: 4 } });
    expect(steppedSize(run(), { sizePt: 144 }, 0.5)).toEqual({ style: { sizePt: 144 } });
  });

  it("toggling back to the run's own face is always allowed", () => {
    expect(toggledFace(run({ face: "italic" }), {}, "italic")).toEqual({
      style: { face: "regular" },
    });
    expect(toggledFace(run({ face: "bold" }), { face: "regular" }, "bold")).toEqual({ style: { face: "bold" } });
  });
});

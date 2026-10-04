// @vitest-environment happy-dom
import { StrictMode, act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { editsSignature, fontsByKey, makeSourceTextObject, type SourceTextObject } from "@/lib/editor";
import { PROBLEM_COPY, UI } from "@/lib/editor/sourceTextCopy";
import type { AppError, TextEditVerdict, TextFont, TextPreview, TextRun } from "@/lib/types";
import type { PageLayout } from "../PageSurface";
import { SourceTextEditor, type SourceTextEditorProps, type TryDone } from "./SourceTextEditor";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** Word-style subset: only the letters "Hello" and "Sağlık" use. */
const SUBSET: TextFont = {
  key: "f1",
  displayName: "Calibri",
  familyHint: "sans",
  embedded: true,
  subset: true,
  alphabet: " !Helo",
  widths: [226, 268, 631, 498, 229, 527],
  wordSpace: true,
};

const RUN: TextRun = {
  id: "t1:fp:0:10-20",
  order: 0,
  line: 0,
  text: "Hello",
  rect: { x: 72, y: 697, w: 25.6, h: 12 },
  origin: { x: 72, y: 700 },
  dir: { x: 1, y: 0 },
  ascent: 9,
  descent: 3,
  caretOffsets: [0, 7.6, 13.5, 16.3, 19.0, 25.4],
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
    originalWidth: 25.4,
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
      bold: { available: false, surface: [] },
      italic: { available: false, surface: [] },
      boldItalic: { available: false, surface: [] },
    },
  },
  substituted: false,
};

const LAYOUT: PageLayout = { cssWidth: 612, cssHeight: 792, geometry: { box: { x: 0, y: 0, w: 612, h: 792 }, rotate: 0, pageIndex: 0 } };

function verdict(extra: Partial<TextEditVerdict>): TextEditVerdict {
  return {
    runId: RUN.id,
    ok: true,
    code: null,
    chars: [],
    reason: null,
    face: null,
    field: null,
    detail: null,
    deltaPt: 0,
    newRect: null,
    caretOffsets: null,
    ...extra,
  };
}

function answer(v: TextEditVerdict, extra: Partial<TextPreview> = {}): TextPreview {
  return { pagePdf: "UERG", verdicts: [v], pageProblem: null, warnings: [], ...extra };
}

let root: Root | null = null;
let host: HTMLElement;
let tryDone: TryDone | null = null;

function mount(extra: Partial<SourceTextEditorProps> = {}, strict = false) {
  const props: SourceTextEditorProps = {
    run: RUN,
    fonts: fontsByKey([SUBSET]),
    layout: LAYOUT,
    pageIndex: 0,
    sourceFingerprint: "fp",
    sourcePageIndex: 0,
    fileName: "invoice.pdf",
    existing: null,
    others: [],
    caret: 2,
    neighbourText: null,
    requestPreview: vi.fn(() => new Promise<TextPreview>(() => {})),
    seedPreview: vi.fn(),
    onApply: vi.fn(),
    onRevert: vi.fn(),
    onClose: vi.fn(),
    onFileError: vi.fn(),
    registerTryDone: (fn) => {
      tryDone = fn;
    },
    ...extra,
  };
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  const editor = createElement(SourceTextEditor, props);
  act(() => root!.render(strict ? createElement(StrictMode, null, editor) : editor));
  return props;
}

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  host.remove();
  tryDone = null;
});

const input = () => host.querySelector<HTMLInputElement>("input.st-chip__input")!;
const message = () => host.querySelector('[role="status"]')!.textContent ?? "";

function type(value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setter.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function press(key: string, init: KeyboardEventInit = {}) {
  act(() => {
    input().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));
  });
}

describe("SourceTextEditor", () => {
  it("opens focused, labelled and described by its message row, caret where the line was clicked", () => {
    mount();
    expect(document.activeElement).toBe(input());
    expect(input().getAttribute("aria-label")).toBe(UI.editor.aria);
    expect(input().getAttribute("aria-describedby")).toBe(host.querySelector('[role="status"]')!.id);
    expect(input().selectionStart).toBe(2);
    expect(input().selectionEnd).toBe(2);
  });

  it("Enter/F2 opening selects all of the text", () => {
    mount({ caret: "all" });
    expect(input().selectionStart).toBe(0);
    expect(input().selectionEnd).toBe(5);
  });

  it("typing ğ into a subset shows the exact missing message and blocks Done", async () => {
    const props = mount();
    type("Hellğ");
    expect(message()).toBe("This document's font can't draw: ğ The file only includes the letters it already uses.");
    press("Enter");
    await act(async () => {});
    expect(props.requestPreview).not.toHaveBeenCalled();
    expect(props.onClose).not.toHaveBeenCalled();
    expect(host.querySelector('[role="status"]')!.getAttribute("aria-live")).toBe("assertive");
  });

  it("a failed preview verdict keeps the editor open with the draft and the PEN_DRIFT copy", async () => {
    const props = mount({ requestPreview: vi.fn().mockResolvedValue(answer(verdict({ ok: false, code: "PEN_DRIFT" }), { pagePdf: null })) });
    type("Hello!");
    press("Enter");
    expect(message()).toBe(UI.status.checking);
    expect(input().readOnly).toBe(true);
    await act(async () => {});
    expect(message()).toBe(PROBLEM_COPY.PEN_DRIFT);
    expect(input().value).toBe("Hello!");
    expect(input().readOnly).toBe(false);
    expect(props.onApply).not.toHaveBeenCalled();
    expect(props.onClose).not.toHaveBeenCalled();
  });

  it("B11: clicking another run while blocked keeps the draft and says so", async () => {
    const props = mount();
    type("Hellğ!");
    act(() => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    await act(async () => {});
    expect(props.onClose).not.toHaveBeenCalled();
    expect(input().value).toBe("Hellğ!");
    expect(message()).toContain(UI.blocked);
    let closed: boolean | undefined;
    await act(async () => {
      closed = await tryDone!({ focusRun: false, blockedNotice: true });
    });
    expect(closed).toBe(false);
  });

  it("Esc cancels the draft: nothing is committed, focus goes back to the line", () => {
    const props = mount();
    type("Help");
    press("Escape");
    expect(props.onApply).not.toHaveBeenCalled();
    expect(props.onClose).toHaveBeenCalledWith({ focusRun: true, announce: null });
  });

  it("empty text shows the removal note", () => {
    mount();
    type("");
    expect(message()).toBe(UI.removal);
  });

  it("ignores Enter and Esc while an IME is composing (isComposing, or WebKit's keyCode 229)", async () => {
    const props = mount();
    type("Hell");
    press("Enter", { isComposing: true });
    press("Enter", { keyCode: 229 } as KeyboardEventInit);
    press("Escape", { isComposing: true });
    await act(async () => {});
    expect(props.requestPreview).not.toHaveBeenCalled();
    expect(props.onClose).not.toHaveBeenCalled();
  });

  it("a size typed in the bar counts even when Done is clicked without leaving the field", async () => {
    const props = mount({ requestPreview: vi.fn().mockResolvedValue(answer(verdict({ newRect: RUN.rect }))) });
    const size = host.querySelector<HTMLInputElement>(`input[aria-label="${UI.bar.fontSize}"]`)!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setter.call(size, "14");
      size.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const done = [...host.querySelectorAll("button")].find((b) => b.textContent?.includes(UI.bar.done))!;
    act(() => done.click());
    await act(async () => {});
    expect(props.requestPreview).toHaveBeenCalledWith([{ runId: RUN.id, originalText: "Hello", text: "Hello", style: { sizePt: 14 } }]);
  });

  it("style changes are ignored while a check is running (the check is for the style it started with)", async () => {
    let finish!: (p: TextPreview) => void;
    const props = mount({ requestPreview: vi.fn(() => new Promise<TextPreview>((res) => (finish = res))) });
    type("Hello!");
    press("Enter");
    press(".", { metaKey: true, shiftKey: true, code: "Period" } as KeyboardEventInit);
    // The draft does not pretend to a size the running check will not commit.
    expect(host.querySelector<HTMLInputElement>(`input[aria-label="${UI.bar.fontSize}"]`)!.value).toBe("12");
    await act(async () => finish(answer(verdict({ newRect: RUN.rect }))));
    expect(props.onApply).toHaveBeenCalledWith(expect.objectContaining({ text: "Hello!", style: {} }));
  });

  it("commit: one preview with the page's other changes, one setSourceText, seeded preview, announcement", async () => {
    const other: SourceTextObject = makeSourceTextObject("o2", 0, RUN.rect, {
      runId: "t1:fp:0:40-50",
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: "Total",
      text: "Sum",
      style: {},
    });
    const newRect = { x: 72, y: 697, w: 27.9, h: 12 };
    const preview = answer(verdict({ deltaPt: 2.5, newRect, caretOffsets: [0, 7.6, 13.5, 16.3, 19, 25.4, 27.9] }));
    const props = mount({ others: [other], requestPreview: vi.fn().mockResolvedValue(preview) });
    type("Hello!");
    press("Enter");
    await act(async () => {});
    expect(props.requestPreview).toHaveBeenCalledTimes(1);
    expect(props.requestPreview).toHaveBeenCalledWith([
      { runId: "t1:fp:0:40-50", originalText: "Total", text: "Sum", style: {} },
      { runId: RUN.id, originalText: "Hello", text: "Hello!", style: {} },
    ]);
    expect(props.onApply).toHaveBeenCalledTimes(1);
    expect(props.onApply).toHaveBeenCalledWith(
      expect.objectContaining({ pageIndex: 0, run: RUN, sourceFingerprint: "fp", sourcePageIndex: 0, text: "Hello!", style: {}, rect: newRect }),
    );
    const committed = makeSourceTextObject("x", 0, newRect, {
      runId: RUN.id,
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: "Hello",
      text: "Hello!",
      style: {},
    });
    expect(props.seedPreview).toHaveBeenCalledWith(editsSignature([other, committed]), preview);
    expect(props.onClose).toHaveBeenCalledWith({ focusRun: true, announce: "Change applied. 2.5 pt wider than before." });
  });

  it("commits under React.StrictMode too (the app's development mode mounts twice)", async () => {
    const preview = answer(verdict({ deltaPt: 1, newRect: RUN.rect, caretOffsets: [0, 7.6, 13.5, 16.3, 19, 25.4, 27.9] }));
    const props = mount({ requestPreview: vi.fn().mockResolvedValue(preview) }, true);
    type("Hello!");
    press("Enter");
    await act(async () => {});
    expect(props.onApply).toHaveBeenCalledTimes(1);
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it("a no-op Done restores the original of a committed change (one revert) and closes", async () => {
    const existing = makeSourceTextObject("e1", 0, RUN.rect, {
      runId: RUN.id,
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: "Hello",
      text: "Help",
      style: {},
    });
    const props = mount({ existing });
    expect(input().value).toBe("Help");
    type("Hello");
    press("Enter");
    await act(async () => {});
    expect(props.requestPreview).not.toHaveBeenCalled();
    expect(props.onRevert).toHaveBeenCalledWith("e1");
    expect(props.onClose).toHaveBeenCalledWith({ focusRun: true, announce: UI.announce.restored });
  });

  it("STALE closes the editor with nothing committed and reports the file", async () => {
    const stale: AppError = { code: "STALE", title: "The PDF changed on disk", message: "m" };
    const props = mount({ requestPreview: vi.fn().mockRejectedValue(stale) });
    type("Hell");
    press("Enter");
    await act(async () => {});
    expect(props.onFileError).toHaveBeenCalledWith(stale);
    expect(props.onApply).not.toHaveBeenCalled();
    expect(props.onClose).toHaveBeenCalledWith({ focusRun: false, announce: null });
  });

  it("⌘I on a font without italic writes the italic sentence; width sentence shows otherwise", () => {
    mount();
    expect(message()).toBe(UI.width.same);
    press("i", { metaKey: true });
    expect(message()).toBe("This page has no italic version of this font.");
    type("Hello!");
    expect(message()).toMatch(/^About \d+(\.\d)? pt wider than before\.$/);
  });
});

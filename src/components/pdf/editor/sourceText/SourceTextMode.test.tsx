// @vitest-environment happy-dom
import { StrictMode, act, createElement, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fontsByKey } from "@/lib/editor";
import { UI, fillCopy } from "@/lib/editor/sourceTextCopy";
import type { PageText, TextFont, TextPreview, TextRun } from "@/lib/types";
import type { PageLayout } from "../PageSurface";
import type { EditSession } from "../useEditSession";
import {
  PreviewStatusChip,
  SourceTextBanners,
  SourceTextMode,
  type SourceTextGuard,
  type SourceTextPage,
  type TextRequest,
} from "./SourceTextMode";
import type { TextPreviewController } from "./useTextPreview";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const FONT: TextFont = {
  key: "f1",
  displayName: "Arial",
  familyHint: "sans",
  embedded: true,
  subset: false,
  alphabet: " !Hdelo",
  widths: [278, 278, 722, 556, 556, 222, 556],
  wordSpace: true,
};
const LAYOUT: PageLayout = { cssWidth: 612, cssHeight: 792, geometry: { box: { x: 0, y: 0, w: 612, h: 792 }, rotate: 0, pageIndex: 0 } };

function run(id: string, order: number, text: string, y: number, editable = true): TextRun {
  return {
    id,
    order,
    line: order,
    text,
    rect: { x: 72, y: y - 3, w: 30, h: 12 },
    origin: { x: 72, y },
    dir: { x: 1, y: 0 },
    ascent: 9,
    descent: 3,
    caretOffsets: Array.from({ length: text.length + 1 }, (_, i) => i * 6),
    editable,
    reason: editable ? null : "ROTATED_TEXT",
    metrics: editable
      ? {
          surface: ["f1"], tfSize: 12, effectiveSize: 12, charSpacing: 0, wordSpacing: 0, hScale: 1, textToUser: 1,
          letterSpacingPt: 0, spaceMode: "glyph", kernSpace: -250, originalWidth: 30, visibleExtent: 500, nextObstacle: null,
        }
      : null,
    style: editable
      ? {
          fill: "#000000", sizeChangeable: true, colourChangeable: true, face: "regular",
          faces: {
            regular: { available: true, surface: ["f1"] }, bold: { available: false, surface: [] },
            italic: { available: false, surface: [] }, boldItalic: { available: false, surface: [] },
          },
        }
      : null,
    substituted: false,
  };
}

const HELLO = run("r1", 0, "Hello", 700);
const TILTED = run("r2", 1, "Rotated", 600, false);

function pageText(runs: TextRun[], pageReason: PageText["pageReason"] = null): PageText {
  return { fingerprint: "fp", pageIndex: 0, pageReason, runs, fonts: [FONT] };
}

function preview(extra: Partial<TextPreviewController> = {}): TextPreviewController {
  return { bytes: null, status: "idle", result: null, error: null, request: vi.fn(), seed: vi.fn(), ...extra };
}

function model(extra: Partial<SourceTextPage> = {}): SourceTextPage {
  return {
    path: "/a.pdf",
    fileName: "a.pdf",
    pageIndex: 0,
    sourcePageIndex: 0,
    active: true,
    duplicate: false,
    source: { status: "ready", info: { fingerprint: "fp", pageCount: 1, warnings: [] }, error: null, stale: false },
    fingerprint: "fp",
    pageText: { page: pageText([HELLO, TILTED]), status: "ready", error: null },
    fonts: fontsByKey([FONT]),
    objects: [],
    staleFingerprint: null,
    preview: preview(),
    canShowOriginal: false,
    problem: null,
    reportFault: vi.fn(),
    ...extra,
  };
}

/** Rendered text (entities decoded). */
function textOf(markup: string): string {
  const div = document.createElement("div");
  div.innerHTML = markup;
  return div.textContent ?? "";
}

const banners = (page: SourceTextPage, dismissed = true) =>
  textOf(
    renderToStaticMarkup(
      <SourceTextBanners page={page} onRemoveStale={() => {}} modeDismissed={dismissed} onDismissMode={() => {}} />,
    ),
  );

describe("SourceTextBanners and PreviewStatusChip", () => {
  it("mode banner until dismissed; page refused, no text, none editable, duplicate page", () => {
    expect(banners(model(), false)).toContain(UI.banner.mode);
    expect(banners(model())).not.toContain(UI.banner.mode);
    expect(banners(model({ pageText: { page: pageText([], "GEOMETRY"), status: "ready", error: null } }))).toContain(
      "Nothing on this page can be changed: This page uses a custom unit size or unreadable page boxes, which OffPDF doesn't edit yet.",
    );
    expect(banners(model({ pageText: { page: pageText([]), status: "ready", error: null } }))).toContain(UI.banner.noText);
    expect(banners(model({ pageText: { page: pageText([TILTED]), status: "ready", error: null } }))).toContain(UI.banner.noneEditable);
    expect(banners(model({ duplicate: true }))).toContain(UI.banner.duplicatePage);
  });

  it("file errors, reading status and the stale banner with its action", () => {
    const encrypted = { code: "ENCRYPTED", title: "t", message: "OffPDF can't change text in a protected PDF.", suggestion: "Remove the password with Unlock PDF, then edit the unlocked copy." };
    const markup = banners(model({ source: { status: "error", info: null, error: encrypted, stale: false }, fingerprint: null }));
    expect(markup).toContain(
      "Edit text is off for “a.pdf”: OffPDF can't change text in a protected PDF. Remove the password with Unlock PDF, then edit the unlocked copy.",
    );
    expect(banners(model({ pageText: { page: null, status: "loading", error: null } }))).toContain(UI.status.reading);
    const stale = banners(model({ staleFingerprint: "fp-old", fingerprint: null }));
    expect(stale).toContain(fillCopy(UI.banner.stale, { name: "a.pdf" }));
    expect(stale).toContain(UI.banner.staleAction);
  });

  it("status chip: checking / showing changes / original / unavailable, only with changes", () => {
    const chip = (status: TextPreviewController["status"], showOriginal = false, objects = 1) =>
      textOf(renderToStaticMarkup(
        <PreviewStatusChip
          page={model({ preview: preview({ status }), objects: Array.from({ length: objects }, () => ({}) as never) })}
          showOriginal={showOriginal}
        />,
      ));
    expect(chip("pending")).toContain(UI.status.checking);
    expect(chip("ready")).toContain(UI.status.showingChanges);
    expect(chip("ready", true)).toContain(UI.status.showingOriginal);
    expect(chip("unavailable")).toContain(UI.status.previewUnavailable);
    expect(chip("ready", false, 0)).toBe("");
  });
});

describe("SourceTextMode", () => {
  let root: Root | null = null;
  let host: HTMLElement;
  afterEach(() => {
    act(() => root?.unmount());
    root = null;
    host.remove();
  });

  function mount(page: SourceTextPage, request: TextRequest | null = null, onRequestHandled = vi.fn()) {
    const session = { setSourceText: vi.fn(), revertSourceText: vi.fn() } as unknown as EditSession;
    const guardRef = createRef<SourceTextGuard | null>() as { current: SourceTextGuard | null };
    const onAddTextHere = vi.fn();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() =>
      root!.render(
        createElement(
          StrictMode,
          null,
          createElement(SourceTextMode, {
            page, layout: LAYOUT, session, stage: null, inert: false, guardRef, request, onRequestHandled, onLeave: () => {}, onAddTextHere,
          }),
        ),
      ),
    );
    return { session, guardRef, onAddTextHere };
  }

  const lineButton = (label: string) =>
    [...host.querySelectorAll<HTMLButtonElement>("button.st-run")].find((b) => b.getAttribute("aria-label")?.includes(label))!;
  const press = (el: Element, key: string) =>
    act(() => {
      el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    });

  it("Enter opens the editor; Done commits once, closes, returns focus to the line and announces", async () => {
    const answer: TextPreview = {
      pagePdf: "UERG",
      verdicts: [{ runId: "r1", ok: true, code: null, chars: [], reason: null, face: null, field: null, detail: null, deltaPt: 1.25, newRect: HELLO.rect, caretOffsets: [0, 6, 12, 18, 24, 30, 33] }],
      pageProblem: null,
      warnings: [],
    };
    const request = vi.fn().mockResolvedValue(answer);
    const { session, guardRef } = mount(model({ preview: preview({ request }) }));
    press(lineButton("Hello"), "Enter");
    const input = host.querySelector<HTMLInputElement>("input.st-chip__input")!;
    expect(input).not.toBeNull();
    expect(guardRef.current?.isEditing()).toBe(true);
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setter.call(input, "Hello!");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    press(input, "Enter");
    await act(async () => {});
    expect(session.setSourceText).toHaveBeenCalledTimes(1);
    expect(host.querySelector("input.st-chip__input")).toBeNull();
    expect(document.activeElement).toBe(lineButton("Hello"));
    expect(host.querySelector(".sr-only")!.textContent).toContain("Change applied. 1.25 pt wider than before.");
    expect(guardRef.current?.isEditing()).toBe(false);
  });

  it("the guard keeps a blocked edit open (page or tool change waits)", async () => {
    const { guardRef } = mount(model());
    press(lineButton("Hello"), "F2");
    const input = host.querySelector<HTMLInputElement>("input.st-chip__input")!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setter.call(input, "Hello ğ");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    let ok: boolean | undefined;
    await act(async () => {
      ok = await guardRef.current!.tryClose();
    });
    expect(ok).toBe(false);
    expect(host.querySelector("input.st-chip__input")).not.toBeNull();
  });

  it("a refused line opens the reason dialog; Esc returns focus to the line; Add text here adds a box", () => {
    const { onAddTextHere } = mount(model());
    press(lineButton("Rotated"), "Enter");
    const dialog = host.querySelector('[role="dialog"]')!;
    expect(dialog.getAttribute("aria-modal")).toBe("false");
    expect(dialog.textContent).toContain(UI.popover.title);
    expect(dialog.textContent).toContain("This line doesn't run straight across the page as shown.");
    expect(document.activeElement?.textContent).toBe(UI.popover.addTextHere);
    press(document.activeElement!, "Escape");
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(lineButton("Rotated"));
    press(lineButton("Rotated"), "Enter");
    act(() => (document.activeElement as HTMLButtonElement).click());
    expect(onAddTextHere).toHaveBeenCalledTimes(1);
    const stamp = onAddTextHere.mock.calls[0][0];
    expect(stamp.fontSize).toBe(12);
    expect(stamp.rect.x).toBeCloseTo(TILTED.rect.x, 6); // over the line, grown to one line × 4 em
    expect(stamp.rect.y + stamp.rect.h).toBeCloseTo(TILTED.rect.y + TILTED.rect.h, 6);
    expect(stamp.rect.w).toBeCloseTo(48, 6);
    expect(stamp.rect.h).toBeCloseTo(15.6, 6);
  });

  it("Add text here on a line turned at an angle adds a default one-line box at its start, not its whole box", () => {
    // Live check: the 35° watermark's axis-aligned box (298 × 260 pt) became the new text box.
    const deg = (35 * Math.PI) / 180;
    const turned: TextRun = { ...TILTED, id: "r3", order: 2, line: 2, text: "CONFIDENTIAL", rect: { x: 150, y: 250, w: 298, h: 260 }, origin: { x: 160, y: 262 }, dir: { x: Math.cos(deg), y: Math.sin(deg) } };
    const { onAddTextHere } = mount(model({ pageText: { page: pageText([HELLO, turned]), status: "ready", error: null } }));
    press(lineButton("CONFIDENTIAL"), "Enter");
    act(() => (document.activeElement as HTMLButtonElement).click());
    const stamp = onAddTextHere.mock.calls[0][0];
    expect(stamp.rect.w).toBeCloseTo(120, 6);
    expect(stamp.rect.h).toBeCloseTo(15.6, 6);
    expect(stamp.rect.x).toBeCloseTo(160, 6);
    expect(stamp.rect.y + stamp.rect.h).toBeCloseTo(274, 6);
  });

  it("a sidebar/inspector request opens that line once and is reported handled (never replayed)", () => {
    const handled = vi.fn();
    mount(model(), { runId: "r1", open: true, tick: 3 }, handled);
    expect(host.querySelector("input.st-chip__input")).not.toBeNull();
    expect(handled).toHaveBeenCalledTimes(1);
  });

  it("acts on a second request in the same mount even when the canvas restarts its tick (focus from the list, then Edit line)", () => {
    // Live check: the canvas clears a handled request and the next one starts again at tick 1,
    // so "select the change in the list" followed by "Edit line" in the inspector did nothing.
    const handled = vi.fn();
    const page = model();
    mount(page, { runId: "r1", open: false, tick: 1 }, handled);
    expect(host.querySelector("input.st-chip__input")).toBeNull();
    const render = (request: TextRequest | null) =>
      act(() =>
        root!.render(
          createElement(
            StrictMode,
            null,
            createElement(SourceTextMode, {
              page, layout: LAYOUT, session: { setSourceText: vi.fn(), revertSourceText: vi.fn() } as unknown as EditSession,
              stage: null, inert: false, guardRef: { current: null }, request, onRequestHandled: handled, onLeave: () => {}, onAddTextHere: vi.fn(),
            }),
          ),
        ),
      );
    render(null);
    render({ runId: "r1", open: true, tick: 1 });
    expect(host.querySelector("input.st-chip__input")).not.toBeNull();
    expect(handled).toHaveBeenCalledTimes(2);
  });

  it("Remove these text changes keeps focus in the editor instead of dropping it on <body>", () => {
    // Live check: the banner and its button disappear with the changes, so focus fell to <body>
    // and ⌘Z (handled only while focus is inside the editor) no longer worked.
    const remove = vi.fn();
    host = document.createElement("div");
    host.className = "pdf-editor";
    host.tabIndex = 0;
    document.body.appendChild(host);
    root = createRoot(host);
    const page = model({ staleFingerprint: "fp-old", fingerprint: null });
    act(() => root!.render(createElement(SourceTextBanners, { page, onRemoveStale: remove, modeDismissed: true, onDismissMode: () => {} })));
    const action = host.querySelector<HTMLButtonElement>(".st-banners__action")!;
    action.focus();
    act(() => action.click());
    act(() => root!.render(createElement(SourceTextBanners, { page: { ...page, staleFingerprint: null }, onRemoveStale: remove, modeDismissed: true, onDismissMode: () => {} })));
    expect(remove).toHaveBeenCalledWith("fp-old");
    expect(document.activeElement).toBe(host);
  });

  it("renders nothing for a refused page (the banner explains)", () => {
    mount(model({ pageText: { page: pageText([HELLO], "PAGE_TOO_COMPLEX"), status: "ready", error: null } }));
    expect(host.querySelector(".st-layer")).toBeNull();
  });
});

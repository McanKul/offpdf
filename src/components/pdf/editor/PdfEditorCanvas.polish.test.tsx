// @vitest-environment happy-dom
// Live-check polish (v0.4 Edit text), through the real canvas, session and toolbar:
// - Show original is offered only when the page shows a real preview of its text changes
//   (it used to toggle between two identical renders when the preview was unavailable);
// - "Add text here" on a line turned at an angle adds a default one-line box at the line's
//   start (it used to add the line's whole axis-aligned box: 298 × 260 pt for a watermark).
import { act, useEffect, useMemo, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PageText, TextPreview, TextRun } from "@/lib/types";
import { UI } from "@/lib/editor/sourceTextCopy";
import { ToastProvider } from "@/components/ui/Toast";
import type { TextSources } from "@/features/edit-pdf/useTextSources";
import type { EditSession } from "./useEditSession";

const LAYOUT = { cssWidth: 612, cssHeight: 792, geometry: { box: { x: 0, y: 0, w: 612, h: 792 }, rotate: 0, pageIndex: 0 } };

vi.mock("./PageSurface", async () => {
  const { useEffect: useMountEffect } = await import("react");
  return {
    PageSurface: ({ onLayout }: { onLayout: (l: typeof LAYOUT) => void }) => {
      useMountEffect(() => onLayout(LAYOUT), []);
      return <canvas />;
    },
  };
});

const HELLO: TextRun = {
  id: "r1",
  order: 0,
  line: 0,
  text: "Hello",
  rect: { x: 72, y: 700, w: 30, h: 12 },
  origin: { x: 72, y: 703 },
  dir: { x: 1, y: 0 },
  ascent: 9,
  descent: 3,
  caretOffsets: [0, 6, 12, 18, 24, 30],
  editable: true,
  reason: null,
  metrics: {
    surface: ["f1"], tfSize: 12, effectiveSize: 12, charSpacing: 0, wordSpacing: 0, hScale: 1, textToUser: 1,
    letterSpacingPt: 0, spaceMode: "glyph", kernSpace: -250, originalWidth: 30, visibleExtent: 500, nextObstacle: null,
  },
  style: {
    fill: "#000000", sizeChangeable: true, colourChangeable: true, face: "regular",
    faces: {
      regular: { available: true, surface: ["f1"] }, bold: { available: false, surface: [] },
      italic: { available: false, surface: [] }, boldItalic: { available: false, surface: [] },
    },
  },
  substituted: false,
};
const DEG = (35 * Math.PI) / 180;
/** A refused 35° watermark near the top of the page (inside the layer's rendered band in happy-dom). */
const WATERMARK: TextRun = {
  ...HELLO,
  id: "r2",
  order: 1,
  line: 1,
  text: "CONFIDENTIAL",
  rect: { x: 150, y: 500, w: 298, h: 260 },
  origin: { x: 160, y: 512 },
  dir: { x: Math.cos(DEG), y: Math.sin(DEG) },
  caretOffsets: [],
  editable: false,
  reason: "ROTATED_TEXT",
  metrics: null,
  style: null,
};
const PAGE: PageText = {
  fingerprint: "fp",
  pageIndex: 0,
  pageReason: null,
  runs: [HELLO, WATERMARK],
  fonts: [{ key: "f1", displayName: "Arial", familyHint: "sans", embedded: true, subset: false, alphabet: " !Hdelo", widths: [278, 278, 722, 556, 556, 222, 556], wordSpace: true }],
};

const previewTextEdits = vi.fn<(...args: unknown[]) => Promise<TextPreview>>();
vi.mock("@/lib/tauriCommands", () => ({
  pagePdf: vi.fn(async () => "JVBERg=="),
  listPdfAnnots: vi.fn(async () => []),
  pickImageFile: vi.fn(async () => null),
  previewImage: vi.fn(),
  inspectTextPage: vi.fn(async () => PAGE),
  previewTextEdits: (...args: unknown[]) => previewTextEdits(...args),
}));

const { PdfEditorCanvas } = await import("./PdfEditorCanvas");
const { useEditSession } = await import("./useEditSession");

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const INFO = { fingerprint: "fp", pageCount: 1, warnings: [] };
const SOURCES: TextSources = {
  sources: { "/a.pdf": { status: "ready", info: INFO, error: null, stale: false } },
  ensure: async () => INFO,
  release: () => {},
  markStale: () => {},
  markError: () => {},
  reopen: async () => INFO,
};

function answer(pagePdf: string | null): TextPreview {
  return {
    pagePdf,
    verdicts: [{ runId: "r1", ok: true, code: null, chars: [], reason: null, face: null, field: null, detail: null, deltaPt: 3, newRect: HELLO.rect, caretOffsets: [0, 6, 12, 18, 24, 30, 33] }],
    pageProblem: null,
    warnings: [],
  };
}

let latest: EditSession | null = null;

function Harness({ withChange }: { withChange: boolean }) {
  const keys = useMemo(() => ["u1#1"], []);
  const session = useEditSession(keys);
  const guardRef = useRef(null);
  const committed = useRef(false);
  latest = session;
  useEffect(() => {
    // Once, like a committed edit.
    if (!withChange || committed.current) return;
    committed.current = true;
    session.setSourceText({ pageIndex: 0, run: HELLO, sourceFingerprint: "fp", sourcePageIndex: 0, text: "Hello!", style: {} });
  }, [withChange, session]);
  return (
    <ToastProvider>
      <PdfEditorCanvas
        sourcePath="/a.pdf"
        sourcePage={1}
        pageIndex={0}
        pageCount={1}
        session={session}
        text={{ sources: SOURCES, fileName: "a.pdf", duplicatePage: false, guardRef }}
      />
    </ToastProvider>
  );
}

let host: HTMLDivElement;
let root: Root | null = null;

async function flush() {
  for (let i = 0; i < 6; i++) await act(async () => {});
}

async function mount(withChange: boolean) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => root?.render(<Harness withChange={withChange} />));
  await flush();
}

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  host.remove();
  latest = null;
  previewTextEdits.mockReset();
});

const showOriginalButton = () =>
  [...host.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === UI.tool.showOriginal.label) ?? null;
const statusChip = () => host.querySelector(".st-chip-status")?.textContent ?? null;
const pressO = () =>
  act(async () => {
    host.querySelector<HTMLElement>(".pdf-editor")?.focus();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "o", bubbles: true }));
  });

describe("Show original is offered only with a preview to compare", () => {
  it("is offered (button and O) when the page shows a preview of its text changes", async () => {
    previewTextEdits.mockResolvedValue(answer("JVBERg=="));
    await mount(true);
    expect(statusChip()).toBe(UI.status.showingChanges);
    expect(showOriginalButton()).not.toBeNull();
    await pressO();
    expect(statusChip()).toBe(UI.status.showingOriginal);
  });

  it("is not offered when the preview is unavailable for the page (nothing to compare)", async () => {
    previewTextEdits.mockResolvedValue(answer(null));
    await mount(true);
    expect(statusChip()).toBe(UI.status.previewUnavailable);
    expect(showOriginalButton()).toBeNull();
    await pressO();
    expect(statusChip()).toBe(UI.status.previewUnavailable);
  });

  it("is not offered while the page has no text changes", async () => {
    await mount(false);
    expect(showOriginalButton()).toBeNull();
    expect(previewTextEdits).not.toHaveBeenCalled();
  });
});

describe("Add text here", () => {
  it("on a line turned at an angle adds a default one-line box at its start, upright, in Add text", async () => {
    await mount(false);
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-keyshortcuts="E"]')?.click());
    await flush();
    const line = [...host.querySelectorAll<HTMLButtonElement>("button.st-run")].find((b) => b.getAttribute("aria-label") === "CONFIDENTIAL");
    expect(line).toBeDefined();
    await act(async () => line?.click());
    const add = [...host.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((b) => b.textContent === UI.popover.addTextHere);
    await act(async () => add?.click());
    await flush();
    const boxes = (latest?.objects ?? []).filter((o) => o.kind === "text");
    expect(boxes).toHaveLength(1);
    const box = boxes[0] as Extract<(typeof boxes)[number], { kind: "text" }>;
    expect(box.fontSize).toBe(12);
    expect(box.rect.w).toBeCloseTo(120, 6); // not 298
    expect(box.rect.h).toBeCloseTo(15.6, 6); // not 260
    expect(box.rect.x).toBeCloseTo(160, 6);
    expect(box.rect.y + box.rect.h).toBeCloseTo(524, 6);
    expect(host.querySelector(`button[title="${UI.tool.addText.title}"]`)?.getAttribute("aria-pressed")).toBe("true");
  });
});

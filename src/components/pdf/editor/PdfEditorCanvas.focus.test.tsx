// @vitest-environment happy-dom
// Live-check regression (v0.4 Edit text): moving focus to the editor root must never scroll the
// window. With a scroll, the first click on a line moved the page between pointerdown and click,
// so the click opened the line below it (or nothing), and Esc on the line layer jumped the page.
import { act, useMemo, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PageText, TextRun } from "@/lib/types";
import { ToastProvider } from "@/components/ui/Toast";
import type { TextSources } from "@/features/edit-pdf/useTextSources";

const LAYOUT = { cssWidth: 612, cssHeight: 792, geometry: { box: { x: 0, y: 0, w: 612, h: 792 }, rotate: 0, pageIndex: 0 } };

vi.mock("./PageSurface", async () => {
  const { useEffect } = await import("react");
  return {
    PageSurface: ({ onLayout }: { onLayout: (l: typeof LAYOUT) => void }) => {
      useEffect(() => onLayout(LAYOUT), []);
      return <canvas />;
    },
  };
});

const RUN: TextRun = {
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
  editable: false,
  reason: "ROTATED_TEXT",
  metrics: null,
  style: null,
  substituted: false,
};
const PAGE: PageText = { fingerprint: "fp", pageIndex: 0, pageReason: null, runs: [RUN], fonts: [] };

vi.mock("@/lib/tauriCommands", () => ({
  pagePdf: vi.fn(async () => "JVBERg=="),
  listPdfAnnots: vi.fn(async () => []),
  pickImageFile: vi.fn(async () => null),
  previewImage: vi.fn(),
  inspectTextPage: vi.fn(async () => PAGE),
  previewTextEdits: vi.fn(),
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

function Harness() {
  const keys = useMemo(() => ["u1#1"], []);
  const session = useEditSession(keys);
  const guardRef = useRef(null);
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
let focusSpy: ReturnType<typeof vi.spyOn>;

async function flush() {
  for (let i = 0; i < 5; i++) await act(async () => {});
}

beforeEach(async () => {
  focusSpy = vi.spyOn(HTMLElement.prototype, "focus");
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => root?.render(<Harness />));
  await flush();
});

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  host.remove();
  focusSpy.mockRestore();
});

function rootFocusCalls(): unknown[][] {
  const editor = host.querySelector(".pdf-editor");
  return focusSpy.mock.calls.filter((_args, i) => focusSpy.mock.contexts[i] === editor);
}

describe("PdfEditorCanvas focus never scrolls the page", () => {
  it("pointerdown on the stage focuses the editor root with preventScroll", async () => {
    const stage = host.querySelector(".pdf-editor__stage");
    expect(stage).not.toBeNull();
    await act(async () => {
      stage?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 }));
    });
    const calls = rootFocusCalls();
    expect(calls.length).toBeGreaterThan(0);
    for (const args of calls) expect(args[0]).toEqual({ preventScroll: true });
  });

  it("Esc on the Edit text layer returns focus to the editor root with preventScroll", async () => {
    const editTool = host.querySelector<HTMLButtonElement>('button[aria-keyshortcuts="E"]');
    expect(editTool).not.toBeNull();
    await act(async () => editTool?.click());
    await flush();
    const line = host.querySelector<HTMLButtonElement>(".st-layer button.st-run");
    expect(line).not.toBeNull();
    focusSpy.mockClear();
    await act(async () => {
      line?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    const calls = rootFocusCalls();
    expect(calls.length).toBe(1);
    expect(calls[0][0]).toEqual({ preventScroll: true });
  });
});

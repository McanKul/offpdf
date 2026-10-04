// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, PageText } from "@/lib/types";

const backend = vi.hoisted(() => ({
  inspectTextPage: vi.fn<(path: string, fp: string, page: number) => Promise<unknown>>(),
}));
vi.mock("@/lib/tauriCommands", () => backend);

import { PAGE_TEXT_CACHE_PAGES, PAGE_TEXT_PREFETCH_MS, lruPut, usePageText, type PageTextArgs, type PageTextState } from "./usePageText";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const page = (pageIndex: number, fingerprint = "fp"): PageText => ({ fingerprint, pageIndex, pageReason: null, runs: [], fonts: [] });

let root: Root | null = null;
let state: PageTextState;
let render: (args: PageTextArgs) => void;

function mount(args: PageTextArgs) {
  function Probe(props: PageTextArgs) {
    state = usePageText(props);
    return null;
  }
  root = createRoot(document.createElement("div"));
  render = (next) => act(() => root!.render(createElement(Probe, next)));
  render(args);
}

const at = (sourcePageIndex: number, extra: Partial<PageTextArgs> = {}): PageTextArgs => ({
  path: "/a.pdf",
  fingerprint: "fp",
  sourcePageIndex,
  enabled: true,
  pageCount: 3,
  ...extra,
});

beforeEach(() => {
  vi.useFakeTimers();
  backend.inspectTextPage.mockReset();
  backend.inspectTextPage.mockImplementation((_p, fp, i) => Promise.resolve(page(i, fp)));
});
afterEach(() => {
  act(() => root?.unmount());
  root = null;
  vi.useRealTimers();
});

describe("usePageText", () => {
  it("is idle (no call) while disabled or without a fingerprint", () => {
    mount(at(0, { enabled: false }));
    expect(state.status).toBe("idle");
    render(at(0, { fingerprint: null }));
    expect(state.status).toBe("idle");
    expect(backend.inspectTextPage).not.toHaveBeenCalled();
  });

  it("reads the page, then prefetches the next one after 300 ms; the next page comes from the cache", async () => {
    mount(at(0));
    expect(state.status).toBe("loading");
    await act(async () => {});
    expect(state.status).toBe("ready");
    expect(state.page?.pageIndex).toBe(0);
    expect(backend.inspectTextPage).toHaveBeenCalledTimes(1);
    await act(async () => {
      vi.advanceTimersByTime(PAGE_TEXT_PREFETCH_MS);
    });
    expect(backend.inspectTextPage).toHaveBeenLastCalledWith("/a.pdf", "fp", 1);
    render(at(1));
    expect(state.status).toBe("ready");
    expect(backend.inspectTextPage).toHaveBeenCalledTimes(2);
  });

  it("leaving the page before 300 ms cancels the prefetch; the last page has nothing to prefetch", async () => {
    mount(at(0));
    await act(async () => {});
    render(at(2));
    await act(async () => {});
    await act(async () => {
      vi.advanceTimersByTime(PAGE_TEXT_PREFETCH_MS * 2);
    });
    expect(backend.inspectTextPage.mock.calls.map((c) => c[2])).toEqual([0, 2]);
  });

  it("drops a late answer for a page the user left", async () => {
    let finish!: (p: PageText) => void;
    backend.inspectTextPage.mockImplementationOnce(() => new Promise((res) => (finish = res)));
    mount(at(0, { pageCount: 1 }));
    render(at(0, { pageCount: 1, fingerprint: "fp2" }));
    await act(async () => {});
    expect(state.page?.fingerprint).toBe("fp2");
    await act(async () => finish(page(0, "fp")));
    expect(state.page?.fingerprint).toBe("fp2");
  });

  it("keeps the backend error (STALE) and rejects an answer for another page", async () => {
    const stale: AppError = { code: "STALE", title: "The PDF changed on disk", message: "m" };
    backend.inspectTextPage.mockRejectedValueOnce(stale);
    mount(at(0, { pageCount: 1 }));
    await act(async () => {});
    expect(state.status).toBe("error");
    expect(state.error).toEqual(stale);

    backend.inspectTextPage.mockResolvedValueOnce(page(5));
    render(at(0, { pageCount: 1, fingerprint: "other" }));
    await act(async () => {});
    expect(state.status).toBe("error");
    expect(state.error?.details).toContain("unexpected answer");
  });

  it("LRU keeps the 16 newest pages", () => {
    const cache = new Map<string, number>();
    for (let i = 0; i < PAGE_TEXT_CACHE_PAGES + 2; i++) lruPut(cache, `k${i}`, i, PAGE_TEXT_CACHE_PAGES);
    expect(cache.size).toBe(PAGE_TEXT_CACHE_PAGES);
    expect(cache.has("k0")).toBe(false);
    expect(cache.has("k1")).toBe(false);
    expect(cache.has(`k${PAGE_TEXT_CACHE_PAGES + 1}`)).toBe(true);
  });
});

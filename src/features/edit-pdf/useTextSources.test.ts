// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppError, TextSourceInfo } from "@/lib/types";

const backend = vi.hoisted(() => ({
  openTextSource: vi.fn<(path: string) => Promise<unknown>>(),
  releaseTextSource: vi.fn<(path: string) => Promise<void>>(),
}));

vi.mock("@/lib/tauriCommands", () => backend);

import { useTextSources, type TextSources, type UseTextSourcesOptions } from "./useTextSources";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const INFO: TextSourceInfo = { fingerprint: "fp-1", pageCount: 3, warnings: [] };
const ENCRYPTED: AppError = {
  code: "ENCRYPTED",
  title: "This PDF is password-protected",
  message: "OffPDF can't change text in a protected PDF.",
  suggestion: "Remove the password with Unlock PDF, then edit the unlocked copy.",
};

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

let root: Root | null = null;
let container: HTMLElement | null = null;
let hook: TextSources;

function mount(options?: UseTextSourcesOptions): void {
  function Probe() {
    hook = useTextSources(options);
    return null;
  }
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => root!.render(createElement(Probe)));
}

function unmount(): void {
  act(() => root?.unmount());
  root = null;
  container?.remove();
  container = null;
}

async function settle<T>(work: () => Promise<T>): Promise<T> {
  let out!: T;
  await act(async () => {
    out = await work();
  });
  return out;
}

beforeEach(() => {
  backend.openTextSource.mockReset();
  backend.releaseTextSource.mockReset();
  backend.releaseTextSource.mockResolvedValue(undefined);
});

afterEach(() => {
  if (root) unmount();
});

describe("useTextSources", () => {
  it("opens a file once and serves the cached info afterwards", async () => {
    const open = deferred<TextSourceInfo>();
    backend.openTextSource.mockReturnValueOnce(open.promise);
    mount();
    let first!: Promise<TextSourceInfo | null>;
    let second!: Promise<TextSourceInfo | null>;
    act(() => {
      first = hook.ensure("/a.pdf");
      second = hook.ensure("/a.pdf");
    });
    expect(hook.sources["/a.pdf"]).toEqual({ status: "loading", info: null, error: null, stale: false });
    open.resolve(INFO);
    expect(await settle(() => first)).toEqual(INFO);
    expect(await settle(() => second)).toEqual(INFO);
    expect(hook.sources["/a.pdf"]).toEqual({ status: "ready", info: INFO, error: null, stale: false });
    expect(await settle(() => hook.ensure("/a.pdf"))).toEqual(INFO);
    expect(backend.openTextSource).toHaveBeenCalledTimes(1);
    expect(backend.openTextSource).toHaveBeenCalledWith("/a.pdf");
  });

  it("keeps the backend error in the state instead of swallowing it", async () => {
    backend.openTextSource.mockRejectedValueOnce(ENCRYPTED);
    mount();
    expect(await settle(() => hook.ensure("/locked.pdf"))).toBeNull();
    expect(hook.sources["/locked.pdf"]).toEqual({ status: "error", info: null, error: ENCRYPTED, stale: false });
    // An error is not retried behind the user's back; reopen does that.
    expect(await settle(() => hook.ensure("/locked.pdf"))).toBeNull();
    expect(backend.openTextSource).toHaveBeenCalledTimes(1);
  });

  it("normalises thrown values and malformed answers to AppErrors", async () => {
    backend.openTextSource.mockRejectedValueOnce(new Error("boom"));
    backend.openTextSource.mockResolvedValueOnce({ fingerprint: "", pageCount: 1, warnings: [] });
    mount();
    await settle(() => hook.ensure("/x.pdf"));
    expect(hook.sources["/x.pdf"].error).toMatchObject({ code: "UNKNOWN", message: "boom" });
    await settle(() => hook.ensure("/y.pdf"));
    expect(hook.sources["/y.pdf"]).toMatchObject({ status: "error", error: { code: "INVALID_RESPONSE" } });
  });

  it("release forgets the file and tells the backend", async () => {
    backend.openTextSource.mockResolvedValueOnce(INFO);
    mount();
    await settle(() => hook.ensure("/a.pdf"));
    act(() => hook.release("/a.pdf"));
    expect(hook.sources["/a.pdf"]).toBeUndefined();
    expect(backend.releaseTextSource).toHaveBeenCalledWith("/a.pdf");
    act(() => hook.release("/never-opened.pdf"));
    expect(backend.releaseTextSource).toHaveBeenCalledTimes(1);
  });

  it("drops a late answer for a released file and releases it again", async () => {
    const open = deferred<TextSourceInfo>();
    backend.openTextSource.mockReturnValueOnce(open.promise);
    mount();
    let pending!: Promise<TextSourceInfo | null>;
    act(() => {
      pending = hook.ensure("/a.pdf");
    });
    act(() => hook.release("/a.pdf"));
    open.resolve(INFO);
    expect(await settle(() => pending)).toBeNull();
    expect(hook.sources["/a.pdf"]).toBeUndefined();
    expect(backend.releaseTextSource).toHaveBeenCalledTimes(2);
  });

  it("reports a failed release through onReleaseError", async () => {
    const onReleaseError = vi.fn();
    backend.openTextSource.mockResolvedValueOnce(INFO);
    backend.releaseTextSource.mockRejectedValueOnce({ ...ENCRYPTED, code: "IO_ERROR" });
    mount({ onReleaseError });
    await settle(() => hook.ensure("/a.pdf"));
    await settle(async () => hook.release("/a.pdf"));
    expect(onReleaseError).toHaveBeenCalledWith("/a.pdf", expect.objectContaining({ code: "IO_ERROR" }));
  });

  it("marks a source stale or failed, and reopen reads it again", async () => {
    backend.openTextSource.mockResolvedValueOnce(INFO);
    backend.openTextSource.mockResolvedValueOnce({ ...INFO, fingerprint: "fp-2" });
    mount();
    await settle(() => hook.ensure("/a.pdf"));
    act(() => hook.markStale("/a.pdf"));
    expect(hook.sources["/a.pdf"]).toMatchObject({ status: "ready", stale: true, info: INFO });
    const repair: AppError = { code: "PDF_NEEDS_REPAIR", title: "t", message: "m" };
    act(() => hook.markError("/a.pdf", repair));
    expect(hook.sources["/a.pdf"]).toMatchObject({ status: "error", error: repair, stale: true });
    const info = await settle(() => hook.reopen("/a.pdf"));
    expect(info?.fingerprint).toBe("fp-2");
    expect(hook.sources["/a.pdf"]).toEqual({ status: "ready", info: { ...INFO, fingerprint: "fp-2" }, error: null, stale: false });
  });

  it("ignores an answer that a reopen superseded", async () => {
    const slow = deferred<TextSourceInfo>();
    backend.openTextSource.mockReturnValueOnce(slow.promise);
    backend.openTextSource.mockResolvedValueOnce({ ...INFO, fingerprint: "fp-new" });
    mount();
    let stale!: Promise<TextSourceInfo | null>;
    act(() => {
      stale = hook.ensure("/a.pdf");
    });
    await settle(() => hook.reopen("/a.pdf"));
    slow.resolve({ ...INFO, fingerprint: "fp-old" });
    expect(await settle(() => stale)).toBeNull();
    expect(hook.sources["/a.pdf"].info?.fingerprint).toBe("fp-new");
    expect(backend.releaseTextSource).not.toHaveBeenCalled();
  });

  it("releases every opened file on unmount", async () => {
    backend.openTextSource.mockResolvedValue(INFO);
    mount();
    await settle(() => hook.ensure("/a.pdf"));
    await settle(() => hook.ensure("/b.pdf"));
    unmount();
    expect(backend.releaseTextSource.mock.calls.map((c) => c[0]).sort()).toEqual(["/a.pdf", "/b.pdf"]);
  });
});

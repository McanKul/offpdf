/**
 * The lines of one source page for Edit text (SPEC §D.4): `inspect_text_page`
 * for the shown page, cached in an LRU of 16 pages keyed
 * `${fingerprint}#${pageIndex}`. The next page is prefetched 300 ms after the
 * shown one is ready; leaving the page before then cancels it. An answer for a
 * page the user already left is dropped.
 */
import { useEffect, useRef, useState } from "react";
import { inspectTextPage } from "@/lib/tauriCommands";
import { toAppError, type AppError, type PageText } from "@/lib/types";

export const PAGE_TEXT_CACHE_PAGES = 16;
export const PAGE_TEXT_PREFETCH_MS = 300;

export interface PageTextArgs {
  path: string | null;
  fingerprint: string | null;
  /** 0-based page in the source file. */
  sourcePageIndex: number | null;
  enabled: boolean;
  /** Source page count; the next page is prefetched only when it exists. */
  pageCount?: number | null;
}

export type PageTextStatus = "idle" | "loading" | "ready" | "error";

export interface PageTextState {
  page: PageText | null;
  status: PageTextStatus;
  error: AppError | null;
}

const IDLE: PageTextState = { page: null, status: "idle", error: null };
const LOADING: PageTextState = { page: null, status: "loading", error: null };

/** Map-backed LRU: reading or writing a key makes it the newest. */
export function lruGet<V>(cache: Map<string, V>, key: string): V | undefined {
  const value = cache.get(key);
  if (value === undefined) return undefined;
  cache.delete(key);
  cache.set(key, value);
  return value;
}

export function lruPut<V>(cache: Map<string, V>, key: string, value: V, max: number): void {
  cache.delete(key);
  cache.set(key, value);
  while (cache.size > max) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
}

/** The answer is for the page that was asked for and has the expected lists. */
function isPageText(value: unknown, fingerprint: string, pageIndex: number): value is PageText {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Partial<PageText>;
  return v.fingerprint === fingerprint && v.pageIndex === pageIndex && Array.isArray(v.runs) && Array.isArray(v.fonts);
}

function unexpectedAnswer(): AppError {
  return { ...toAppError(null), details: "inspect_text_page returned an unexpected answer" };
}

export function usePageText({ path, fingerprint, sourcePageIndex, enabled, pageCount }: PageTextArgs): PageTextState {
  const cache = useRef(new Map<string, PageText>());
  const [state, setState] = useState<PageTextState & { key: string | null }>({ ...IDLE, key: null });
  const key = enabled && path && fingerprint && sourcePageIndex !== null ? `${fingerprint}#${sourcePageIndex}` : null;

  useEffect(() => {
    if (!key || !path || !fingerprint || sourcePageIndex === null) {
      setState({ ...IDLE, key: null });
      return;
    }
    const hit = lruGet(cache.current, key);
    if (hit) {
      setState({ page: hit, status: "ready", error: null, key });
      return;
    }
    let active = true;
    setState({ ...LOADING, key });
    inspectTextPage(path, fingerprint, sourcePageIndex).then(
      (page) => {
        if (!active) return;
        if (!isPageText(page, fingerprint, sourcePageIndex)) {
          setState({ page: null, status: "error", error: unexpectedAnswer(), key });
          return;
        }
        lruPut(cache.current, key, page, PAGE_TEXT_CACHE_PAGES);
        setState({ page, status: "ready", error: null, key });
      },
      (e: unknown) => {
        if (active) setState({ page: null, status: "error", error: toAppError(e), key });
      },
    );
    return () => {
      active = false;
    };
  }, [key, path, fingerprint, sourcePageIndex]);

  const ready = state.key === key && state.status === "ready";
  useEffect(() => {
    if (!ready || !path || !fingerprint || sourcePageIndex === null || pageCount == null) return;
    const next = sourcePageIndex + 1;
    const nextKey = `${fingerprint}#${next}`;
    if (next >= pageCount || cache.current.has(nextKey)) return;
    const timer = setTimeout(() => {
      inspectTextPage(path, fingerprint, next).then(
        (page) => {
          if (isPageText(page, fingerprint, next)) lruPut(cache.current, nextKey, page, PAGE_TEXT_CACHE_PAGES);
        },
        () => {
          // Dropped on purpose: a prefetch is a guess. When the user opens that
          // page, the same request runs again and its error is shown there.
        },
      );
    }, PAGE_TEXT_PREFETCH_MS);
    return () => clearTimeout(timer);
  }, [ready, path, fingerprint, sourcePageIndex, pageCount]);

  if (!key) return IDLE;
  if (state.key !== key) return LOADING;
  return { page: state.page, status: state.status, error: state.error };
}

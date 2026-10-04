/**
 * Per-file Edit text sources (SPEC §D.4), owned by `EditPdfPage` like form fields.
 *
 * `ensure` opens a file once (`open_text_source`, deduplicated while in flight),
 * `release` forgets it and lets the backend delete its temporary copies,
 * `markStale` / `markError` record what a later inspect or preview reported, and
 * `reopen` reads the file again. Every failure is kept in the state (or passed
 * to `onReleaseError`); nothing is swallowed. Late answers for a path that was
 * released or reopened meanwhile are dropped by a per-path generation counter.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { openTextSource, releaseTextSource } from "@/lib/tauriCommands";
import { toAppError, type AppError, type TextSourceInfo } from "@/lib/types";

export interface TextSourceState {
  status: "idle" | "loading" | "ready" | "error";
  info: TextSourceInfo | null;
  error: AppError | null;
  stale: boolean;
}

export interface UseTextSourcesOptions {
  /** A `release_text_source` failure (the file is already gone from the state). */
  onReleaseError?: (path: string, error: AppError) => void;
}

export interface TextSources {
  sources: Record<string, TextSourceState>;
  ensure(path: string): Promise<TextSourceInfo | null>;
  release(path: string): void;
  markStale(path: string): void;
  markError(path: string, error: AppError): void;
  reopen(path: string): Promise<TextSourceInfo | null>;
}

const INVALID_ANSWER: AppError = {
  code: "INVALID_RESPONSE",
  title: "Edit text could not start",
  message: "OffPDF received an unexpected answer while reading this PDF for Edit text.",
  suggestion: "Close the file and add it again.",
  details: null,
};

function isSourceInfo(value: unknown): value is TextSourceInfo {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.fingerprint === "string" &&
    v.fingerprint.length > 0 &&
    typeof v.pageCount === "number" &&
    Number.isInteger(v.pageCount) &&
    v.pageCount >= 0 &&
    Array.isArray(v.warnings) &&
    v.warnings.every((w) => typeof w === "string")
  );
}

export function useTextSources(options: UseTextSourcesOptions = {}): TextSources {
  const [sources, setSources] = useState<Record<string, TextSourceState>>({});
  const sourcesRef = useRef<Record<string, TextSourceState>>({});
  const inflight = useRef(new Map<string, Promise<TextSourceInfo | null>>());
  const generations = useRef(new Map<string, number>());
  const mounted = useRef(true);
  const onReleaseErrorRef = useRef(options.onReleaseError);
  onReleaseErrorRef.current = options.onReleaseError;

  const write = useCallback((path: string, next: TextSourceState | null) => {
    const { [path]: _previous, ...rest } = sourcesRef.current;
    sourcesRef.current = next ? { ...rest, [path]: next } : rest;
    if (mounted.current) setSources(sourcesRef.current);
  }, []);

  const bump = useCallback((path: string): number => {
    const generation = (generations.current.get(path) ?? 0) + 1;
    generations.current.set(path, generation);
    return generation;
  }, []);

  const releaseOnBackend = useCallback((path: string) => {
    releaseTextSource(path).catch((e: unknown) => onReleaseErrorRef.current?.(path, toAppError(e)));
  }, []);

  const open = useCallback(
    (path: string): Promise<TextSourceInfo | null> => {
      const generation = bump(path);
      write(path, { status: "loading", info: null, error: null, stale: false });
      const isCurrent = () => generations.current.get(path) === generation && mounted.current;
      const request = openTextSource(path).then(
        (info) => {
          if (!isCurrent()) {
            // Released while reading: the backend cached a snapshot nobody will release.
            if (!(path in sourcesRef.current) || !mounted.current) releaseOnBackend(path);
            return null;
          }
          if (!isSourceInfo(info)) {
            write(path, { status: "error", info: null, error: INVALID_ANSWER, stale: false });
            return null;
          }
          write(path, { status: "ready", info, error: null, stale: false });
          return info;
        },
        (e: unknown) => {
          if (isCurrent()) write(path, { status: "error", info: null, error: toAppError(e), stale: false });
          return null;
        },
      );
      const tracked = request.finally(() => {
        if (inflight.current.get(path) === tracked) inflight.current.delete(path);
      });
      inflight.current.set(path, tracked);
      return tracked;
    },
    [bump, releaseOnBackend, write],
  );

  const ensure = useCallback(
    (path: string): Promise<TextSourceInfo | null> => {
      const current = sourcesRef.current[path];
      if (current?.status === "ready" && current.info) return Promise.resolve(current.info);
      if (current?.status === "error") return Promise.resolve(null);
      const pending = inflight.current.get(path);
      if (pending) return pending;
      return open(path);
    },
    [open],
  );

  const reopen = useCallback(
    (path: string): Promise<TextSourceInfo | null> => {
      inflight.current.delete(path);
      return open(path);
    },
    [open],
  );

  const release = useCallback(
    (path: string) => {
      const known = path in sourcesRef.current || inflight.current.has(path);
      bump(path);
      inflight.current.delete(path);
      write(path, null);
      if (known) releaseOnBackend(path);
    },
    [bump, releaseOnBackend, write],
  );

  const markStale = useCallback(
    (path: string) => {
      const current = sourcesRef.current[path] ?? { status: "idle", info: null, error: null, stale: false };
      if (!current.stale) write(path, { ...current, stale: true });
    },
    [write],
  );

  const markError = useCallback(
    (path: string, error: AppError) => {
      bump(path);
      inflight.current.delete(path);
      const current = sourcesRef.current[path];
      write(path, { status: "error", info: current?.info ?? null, error, stale: current?.stale ?? false });
    },
    [bump, write],
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const paths = new Set([...Object.keys(sourcesRef.current), ...inflight.current.keys()]);
      for (const path of paths) {
        bump(path);
        releaseOnBackend(path);
      }
      inflight.current.clear();
      sourcesRef.current = {};
    };
  }, [bump, releaseOnBackend]);

  return { sources, ensure, release, markStale, markError, reopen };
}

/**
 * Lazily renders + caches page thumbnails for a file. Only tiny PNG data URLs
 * are kept in memory; the source PDF bytes never enter the webview. Rendering is
 * chunked so thumbnails appear progressively for big documents.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { renderThumbnails } from "@/lib/tauriCommands";
import { toAppError, type FileInfo } from "@/lib/types";
import { useToast } from "@/components/ui/Toast";

const CHUNK = 8;

export function useThumbnails(file: FileInfo | undefined, size = 240) {
  const [urls, setUrls] = useState<Record<number, string>>({});
  const urlsRef = useRef<Record<number, string>>({});
  const loadingRef = useRef<Set<number>>(new Set());
  const [, force] = useState(0);
  const { toast } = useToast();

  const apply = useCallback((updater: (prev: Record<number, string>) => Record<number, string>) => {
    setUrls((prev) => {
      const next = updater(prev);
      urlsRef.current = next;
      return next;
    });
  }, []);

  // Reset the cache when the file changes.
  useEffect(() => {
    urlsRef.current = {};
    loadingRef.current = new Set();
    setUrls({});
  }, [file?.path]);

  const ensure = useCallback(
    async (pages: number[]) => {
      if (!file) return;
      const need = pages.filter(
        (p) => urlsRef.current[p] === undefined && !loadingRef.current.has(p),
      );
      if (need.length === 0) return;

      need.forEach((p) => loadingRef.current.add(p));
      force((n) => n + 1);

      try {
        for (let i = 0; i < need.length; i += CHUNK) {
          const slice = need.slice(i, i + CHUNK);
          const res = await renderThumbnails(file.path, slice, size);
          apply((prev) => {
            const next = { ...prev };
            for (const r of res) next[r.page] = r.dataUrl;
            return next;
          });
          slice.forEach((p) => loadingRef.current.delete(p));
          force((n) => n + 1);
        }
      } catch (e) {
        need.forEach((p) => loadingRef.current.delete(p));
        force((n) => n + 1);
        toast({
          title: "Could not render preview",
          description: toAppError(e).message,
          variant: "error",
        });
      }
    },
    [file, size, apply, toast],
  );

  const isLoading = useCallback((p: number) => loadingRef.current.has(p), []);

  return { urls, ensure, isLoading };
}

/** The thumbnail API shared between PagePicker and its grids. */
export type ThumbApi = ReturnType<typeof useThumbnails>;

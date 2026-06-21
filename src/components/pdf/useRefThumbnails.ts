/** Thumbnail cache across multiple files, keyed by each page ref's unique
 * `key` (uid#page). Renders lazily in chunks so big combined documents fill in
 * progressively. The cache is module-level, so it survives tab/tool switches. */
import { useCallback, useRef, useState } from "react";
import { renderThumbnails } from "@/lib/tauriCommands";
import type { PageRef } from "@/lib/types";

const cache = new Map<string, string>();
const CHUNK = 8;

export function useRefThumbnails(size = 240) {
  const [, force] = useState(0);
  const loading = useRef<Set<string>>(new Set());

  const ensure = useCallback(
    async (refs: PageRef[]) => {
      const need = refs.filter((r) => cache.get(r.key) === undefined && !loading.current.has(r.key));
      if (need.length === 0) return;

      need.forEach((r) => loading.current.add(r.key));
      force((n) => n + 1);

      // Group needed refs by source file, but keep the refs so results map back
      // to each ref's own key (two refs can share a path → same rendered page).
      const byPath = new Map<string, PageRef[]>();
      for (const r of need) {
        const arr = byPath.get(r.path) ?? [];
        arr.push(r);
        byPath.set(r.path, arr);
      }

      for (const [path, group] of byPath) {
        for (let i = 0; i < group.length; i += CHUNK) {
          const slice = group.slice(i, i + CHUNK);
          const pages = Array.from(new Set(slice.map((r) => r.page)));
          try {
            const res = await renderThumbnails(path, pages, size);
            const urlByPage = new Map(res.map((t) => [t.page, t.dataUrl]));
            for (const r of slice) {
              const u = urlByPage.get(r.page);
              if (u) cache.set(r.key, u);
            }
          } catch {
            // leave uncached; UI shows a numbered placeholder
          } finally {
            slice.forEach((r) => loading.current.delete(r.key));
            force((n) => n + 1);
          }
        }
      }
    },
    [size],
  );

  const get = useCallback((key: string) => cache.get(key), []);
  const isLoading = useCallback((key: string) => loading.current.has(key), []);

  return { ensure, get, isLoading };
}

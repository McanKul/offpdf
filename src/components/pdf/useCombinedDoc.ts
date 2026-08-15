/** The whole workspace as one logical document: every page of every loaded
 * file, in file order. Used by the cross-document arrange tools. */
import { useMemo } from "react";
import { useWorkspace } from "@/state/workspaceStore";
import type { PageGroup, PagePick, PageRef, WorkspaceFile } from "@/lib/types";

/** Stable page identity: `${file.uid}#${1-based page}`. */
export function pageKeysForFiles(files: WorkspaceFile[]): string[] {
  const keys: string[] = [];
  for (const f of files) {
    const n = f.pageCount ?? 0;
    for (let i = 1; i <= n; i++) keys.push(`${f.uid}#${i}`);
  }
  return keys;
}

export function useCombinedDoc(): PageRef[] {
  const files = useWorkspace((s) => s.files);
  return useMemo(
    () =>
      files.flatMap((f) => {
        const n = f.pageCount ?? 0;
        return Array.from({ length: n }, (_, i) => ({
          // Keyed by the file's unique id so the same file added twice (or two
          // images that map to the same temp PDF) never collide.
          key: `${f.uid}#${i + 1}`,
          path: f.path,
          page: i + 1,
          fileName: f.name,
        }));
      }),
    [files],
  );
}

/** Group an ordered list of page refs into qpdf `--pages` groups (consecutive
 * pages from the same file are merged into one group; order is preserved). */
export function buildGroups(list: PageRef[]): PageGroup[] {
  const groups: { path: string; pages: number[] }[] = [];
  for (const r of list) {
    const last = groups[groups.length - 1];
    if (last && last.path === r.path) last.pages.push(r.page);
    else groups.push({ path: r.path, pages: [r.page] });
  }
  return groups.map((g) => ({ path: g.path, pages: g.pages.join(",") }));
}

/** Flatten ordered page refs into per-page picks (for compress / split). */
export function buildPicks(list: PageRef[]): PagePick[] {
  return list.map((r) => ({ path: r.path, page: r.page }));
}

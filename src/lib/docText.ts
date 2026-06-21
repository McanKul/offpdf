/** Shared, memory-safe full-text cache for in-app search. Text is extracted
 * natively (pdftotext, capped at 64 MB in Rust); only text strings are held
 * here — never PDF bytes. Used by both the inline PdfSearch and the viewer. */
import { pdfText } from "@/lib/tauriCommands";
import type { PageRef } from "@/lib/types";

const cache = new Map<string, string[]>();
const loaded = new Set<string>();

/** Extract+cache text for any of these paths not seen yet. */
export async function ensureText(paths: string[]): Promise<void> {
  const todo = Array.from(new Set(paths)).filter((p) => !loaded.has(p));
  for (const p of todo) {
    try {
      cache.set(p, await pdfText(p));
    } catch {
      cache.set(p, []);
    }
    loaded.add(p);
  }
}

/** Text of one page (1-based), or "" if not cached / no text. */
export function pageText(path: string, page: number): string {
  return cache.get(path)?.[page - 1] ?? "";
}

/** Whether all the given paths' text has been loaded. */
export function isLoaded(paths: string[]): boolean {
  return Array.from(new Set(paths)).every((p) => loaded.has(p));
}

/** Whether any page in the set has extractable text (false → likely scanned). */
export function hasAnyText(refs: PageRef[]): boolean {
  return refs.some((r) => pageText(r.path, r.page).trim().length > 0);
}

/** Override the cached text for a page key set (used after on-the-fly OCR). */
export function setText(path: string, pages: string[]): void {
  cache.set(path, pages);
  loaded.add(path);
}

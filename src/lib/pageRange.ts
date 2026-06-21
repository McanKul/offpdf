/**
 * Robust page-range parser shared by the extract / delete / rotate / reorder /
 * split tools. Mirrors the validation in `src-tauri/src/pdf_engine` so the UI
 * can give immediate feedback before any command is invoked.
 *
 * Accepts: "1", "1,2,3", "1-5", "1,3,5-8", "all". Whitespace is ignored.
 * Descending ranges expand in reverse: "5-3" -> [5,4,3] (useful for reorder).
 *
 * Rules:
 *  - page numbers are 1-based; 0 and negatives are rejected
 *  - if `pageCount` is given, pages beyond it are rejected
 *  - default mode sorts ascending and removes duplicates
 *  - `preserveOrder` (reorder mode) keeps the exact order and allows duplicates
 */

export type ParseResult =
  | { ok: true; pages: number[] }
  | { ok: false; error: string };

export interface ParseOptions {
  /** Upper bound for validation and for expanding the `all` keyword. */
  pageCount?: number;
  /** Reorder mode: preserve the user's exact order and keep duplicates. */
  preserveOrder?: boolean;
  /** Accept the literal `all` keyword (default true). */
  allowAll?: boolean;
}

const ALL_KEYWORDS = new Set(["all", "*"]);

export function parsePageRange(
  input: string,
  opts: ParseOptions = {},
): ParseResult {
  const { pageCount, preserveOrder = false, allowAll = true } = opts;

  const normalized = input.replace(/\s+/g, "").toLowerCase();

  if (normalized.length === 0) {
    return { ok: false, error: "Enter at least one page or range (e.g. 1,3,5-8)." };
  }

  // "all" keyword.
  if (ALL_KEYWORDS.has(normalized)) {
    if (!allowAll) {
      return { ok: false, error: "“all” is not allowed for this field." };
    }
    if (!pageCount || pageCount < 1) {
      return {
        ok: false,
        error: "“all” needs a valid PDF so the page count is known.",
      };
    }
    return { ok: true, pages: range(1, pageCount) };
  }

  const tokens = normalized.split(",");
  const pages: number[] = [];

  for (const token of tokens) {
    if (token.length === 0) {
      return { ok: false, error: "Empty value between commas. Check for a stray “,”." };
    }

    if (token.includes("-")) {
      const parts = token.split("-");
      if (parts.length !== 2 || parts[0] === "" || parts[1] === "") {
        return { ok: false, error: `“${token}” is not a valid range. Use e.g. 5-8.` };
      }
      const start = toPositiveInt(parts[0]);
      const end = toPositiveInt(parts[1]);
      if (start === null || end === null) {
        return { ok: false, error: `“${token}” must use whole page numbers (1 or higher).` };
      }
      const overflow = checkBounds([start, end], pageCount);
      if (overflow) return overflow;
      for (const p of range(start, end)) pages.push(p);
    } else {
      const single = toPositiveInt(token);
      if (single === null) {
        return { ok: false, error: `“${token}” is not a valid page number (use 1 or higher).` };
      }
      const overflow = checkBounds([single], pageCount);
      if (overflow) return overflow;
      pages.push(single);
    }
  }

  if (preserveOrder) {
    return { ok: true, pages };
  }

  const unique = Array.from(new Set(pages)).sort((a, b) => a - b);
  return { ok: true, pages: unique };
}

/** Compact a sorted/ordered list back into a qpdf-friendly string. */
export function formatPageList(pages: number[]): string {
  if (pages.length === 0) return "";
  const out: string[] = [];
  let start = pages[0];
  let prev = pages[0];

  for (let i = 1; i < pages.length; i++) {
    const cur = pages[i];
    if (cur === prev + 1) {
      prev = cur;
      continue;
    }
    out.push(start === prev ? `${start}` : `${start}-${prev}`);
    start = cur;
    prev = cur;
  }
  out.push(start === prev ? `${start}` : `${start}-${prev}`);
  return out.join(",");
}

/** Pages to keep when deleting `toDelete` from a document of `pageCount` pages. */
export function invertPages(toDelete: number[], pageCount: number): number[] {
  const remove = new Set(toDelete);
  const keep: number[] = [];
  for (let p = 1; p <= pageCount; p++) {
    if (!remove.has(p)) keep.push(p);
  }
  return keep;
}

// ---- helpers --------------------------------------------------------------

function toPositiveInt(value: string): number | null {
  if (!/^\d+$/.test(value)) return null;
  const n = Number(value);
  if (!Number.isInteger(n) || n < 1) return null;
  return n;
}

function checkBounds(values: number[], pageCount?: number): ParseResult | null {
  if (!pageCount) return null;
  for (const v of values) {
    if (v > pageCount) {
      return {
        ok: false,
        error: `Page ${v} is out of range — this PDF has ${pageCount} page${
          pageCount === 1 ? "" : "s"
        }.`,
      };
    }
  }
  return null;
}

/** Inclusive integer range; expands in reverse when `start > end`. */
function range(start: number, end: number): number[] {
  const out: number[] = [];
  if (start <= end) {
    for (let i = start; i <= end; i++) out.push(i);
  } else {
    for (let i = start; i >= end; i--) out.push(i);
  }
  return out;
}

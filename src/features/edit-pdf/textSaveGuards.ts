/**
 * Save guards for text changes (SPEC §D.8), checked before `edit_pdf_overlays`
 * runs. Rust repeats every check; these only turn the predictable failures
 * into an immediate toast. Order: source not ready, stale, redaction on the
 * same page, edited page listed twice.
 */

import type { EditObject, SourceTextObject } from "@/lib/editor";
import { FILE_ERROR_COPY, SAVE_GUARD_COPY, fillCopy } from "@/lib/editor/sourceTextCopy";
import type { PageRef } from "@/lib/types";
import type { TextSourceState } from "./useTextSources";

export interface TextSaveGuardInput {
  objects: EditObject[];
  refs: PageRef[];
  sources: Record<string, TextSourceState>;
}

export interface TextSaveGuardResult {
  title: string;
  description: string;
}

const UNKNOWN_FILE = "this PDF";

function sourceTextObjects(objects: EditObject[]): SourceTextObject[] {
  return objects.filter((o): o is SourceTextObject => o.kind === "sourceText");
}

function rowWithSuggestion(code: string, vars: Record<string, string | number>): TextSaveGuardResult {
  const row = FILE_ERROR_COPY[code];
  return {
    title: row.title,
    description: `${fillCopy(row.message, vars)} ${row.suggestion}`.trim(),
  };
}

function notReady(edits: SourceTextObject[], input: TextSaveGuardInput): TextSaveGuardResult | null {
  for (const o of edits) {
    const ref = input.refs[o.pageIndex];
    const source = ref ? input.sources[ref.path] : undefined;
    if (!ref || !source || source.status !== "ready" || !source.info) {
      return {
        title: SAVE_GUARD_COPY.notReadyTitle,
        description: fillCopy(SAVE_GUARD_COPY.notReadyDescription, { name: ref?.fileName ?? UNKNOWN_FILE }),
      };
    }
  }
  return null;
}

function stale(edits: SourceTextObject[], input: TextSaveGuardInput): TextSaveGuardResult | null {
  for (const o of edits) {
    const ref = input.refs[o.pageIndex];
    const source = input.sources[ref.path];
    const moved = ref.page - 1 !== o.sourcePageIndex;
    if (source.stale || source.info?.fingerprint !== o.sourceFingerprint || moved) {
      const row = FILE_ERROR_COPY.STALE;
      return { title: row.title, description: fillCopy(row.message, { name: ref.fileName }) };
    }
  }
  return null;
}

function redactionConflict(edits: SourceTextObject[], input: TextSaveGuardInput): TextSaveGuardResult | null {
  const redacted = new Set(input.objects.filter((o) => o.kind === "redact").map((o) => o.pageIndex));
  const hit = edits.find((o) => redacted.has(o.pageIndex));
  return hit ? rowWithSuggestion("TEXT_EDIT_ON_REDACTED_PAGE", { n: hit.pageIndex + 1 }) : null;
}

function duplicatePage(edits: SourceTextObject[], input: TextSaveGuardInput): TextSaveGuardResult | null {
  const listed = new Map<string, number>();
  for (const ref of input.refs) {
    const key = `${ref.path}\u0000${ref.page}`;
    listed.set(key, (listed.get(key) ?? 0) + 1);
  }
  for (const o of edits) {
    const ref = input.refs[o.pageIndex];
    if ((listed.get(`${ref.path}\u0000${ref.page}`) ?? 0) > 1) {
      return rowWithSuggestion("TEXT_EDIT_DUPLICATE_PAGE", { p: ref.page, name: ref.fileName });
    }
  }
  return null;
}

/** The first reason the text changes can't be saved yet, or null when all clear. */
export function textSaveGuard(input: TextSaveGuardInput): TextSaveGuardResult | null {
  const edits = sourceTextObjects(input.objects);
  if (edits.length === 0) return null;
  return (
    notReady(edits, input) ?? stale(edits, input) ?? redactionConflict(edits, input) ?? duplicatePage(edits, input)
  );
}

/** The open line edit of the Edit text layer (SourceTextMode's guard), seen from Save. */
export interface OpenTextEdit {
  isEditing(): boolean;
  tryClose(): Promise<boolean>;
}

/**
 * Save must never snapshot the objects while a line edit is still open or being checked: the
 * click on Save also commits that edit (outside click, B11), so the saved file would miss a
 * change the editor then shows as applied. "none": nothing open; "finished": the edit was
 * committed (or dropped as a no-op), so read the objects again before saving; "blocked": the
 * draft can't be applied and stays open with its message.
 */
export async function finishOpenTextEdit(edit: OpenTextEdit | null): Promise<"none" | "finished" | "blocked"> {
  if (!edit?.isEditing()) return "none";
  return (await edit.tryClose()) ? "finished" : "blocked";
}

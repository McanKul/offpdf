/** Input validation + disk-space estimation shared across tools. */

import type { RotationAngle, ToolId } from "./types";

/**
 * Estimate the temporary + output disk space a job needs, per the project
 * spec. The native side does the authoritative `check_disk_space`; this gives
 * the frontend a number to send and to warn against.
 *
 *  - merge:    sum(inputs) × 1.5
 *  - optimize: input × 2.5
 *  - split / extract / delete / rotate / reorder: input × 2
 */
export function estimateRequiredBytes(tool: ToolId, inputSizesBytes: number[]): number {
  const total = inputSizesBytes.reduce((a, b) => a + b, 0);
  const multiplier = tool === "merge" ? 1.5 : tool === "optimize" ? 2.5 : 2;
  return Math.ceil(total * multiplier);
}

/** Validate a rotation angle coming from the UI. */
export function isValidAngle(angle: number): angle is RotationAngle {
  return angle === 90 || angle === 180 || angle === 270;
}

// Characters not allowed in a Windows file name, plus control chars.
const INVALID_NAME_CHARS = new RegExp('[<>:"/\\\\|?*\\x00-\\x1f]');
const RESERVED_WINDOWS_NAMES = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.|$)/i;

export type NameValidation = { ok: true; value: string } | { ok: false; error: string };

/**
 * Validate a user-provided output file name (without directory). Ensures it is
 * non-empty, has no path separators or reserved characters, and ends in .pdf.
 */
export function validateOutputName(name: string): NameValidation {
  const trimmed = name.trim();
  if (trimmed.length === 0) {
    return { ok: false, error: "Enter an output file name." };
  }
  if (INVALID_NAME_CHARS.test(trimmed)) {
    return {
      ok: false,
      error: 'A file name cannot contain: < > : " / \\ | ? *',
    };
  }
  if (RESERVED_WINDOWS_NAMES.test(trimmed)) {
    return { ok: false, error: `“${trimmed}” is a reserved name on Windows.` };
  }
  if (trimmed.length > 200) {
    return { ok: false, error: "That file name is too long." };
  }
  const withExt = /\.pdf$/i.test(trimmed) ? trimmed : `${trimmed}.pdf`;
  return { ok: true, value: withExt };
}

/** Join a directory and file name with the platform-appropriate separator. */
export function joinPath(dir: string, name: string): string {
  if (!dir) return name;
  const sep = dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
  const trimmedDir = dir.replace(/[\\/]+$/, "");
  return `${trimmedDir}${sep}${name}`;
}

/** At least one PDF was selected. */
export function requirePdfSelected(count: number): string | null {
  return count > 0 ? null : "Select at least one PDF file to continue.";
}

/** An output folder was chosen. */
export function requireOutputFolder(folder: string | null): string | null {
  return folder ? null : "Choose an output folder.";
}

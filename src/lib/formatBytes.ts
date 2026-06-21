/** Human-friendly byte / number / time formatting helpers. */

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

/** Format a byte count, e.g. 2_500_000_000 -> "2.33 GB". Base-1024. */
export function formatBytes(bytes: number, decimals = 2): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes === 0) return "0 B";

  const i = Math.min(
    UNITS.length - 1,
    Math.floor(Math.log(bytes) / Math.log(1024)),
  );
  const value = bytes / Math.pow(1024, i);
  // No decimals for plain bytes.
  const d = i === 0 ? 0 : decimals;
  return `${value.toFixed(d)} ${UNITS[i]}`;
}

/** Size thresholds used for large-file warnings. */
export const LARGE_FILE_BYTES = 500 * 1024 * 1024; // 500 MB
export const VERY_LARGE_FILE_BYTES = 2 * 1024 * 1024 * 1024; // 2 GB

export type FileSizeTier = "normal" | "large" | "veryLarge";

export function fileSizeTier(bytes: number): FileSizeTier {
  if (bytes >= VERY_LARGE_FILE_BYTES) return "veryLarge";
  if (bytes >= LARGE_FILE_BYTES) return "large";
  return "normal";
}

/** Format a count with thousands separators, e.g. 1234 -> "1,234". */
export function formatCount(n: number): string {
  return n.toLocaleString("en-US");
}

/** Relative time like "just now", "5 min ago", "2 days ago". */
export function formatRelativeTime(epochMs: number, now = Date.now()): string {
  const diff = Math.max(0, now - epochMs);
  const sec = Math.floor(diff / 1000);
  if (sec < 45) return "just now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min} min ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr} hour${hr === 1 ? "" : "s"} ago`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `${day} day${day === 1 ? "" : "s"} ago`;
  return new Date(epochMs).toLocaleDateString();
}

/** Basename of a path (handles both / and \\ separators). */
export function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** Directory portion of a path. */
export function dirname(path: string): string {
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return idx >= 0 ? path.slice(0, idx) : "";
}

/** Strip the extension from a filename, e.g. "plan.pdf" -> "plan". */
export function stripExt(name: string): string {
  const idx = name.lastIndexOf(".");
  return idx > 0 ? name.slice(0, idx) : name;
}

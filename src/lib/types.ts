/**
 * Shared types — the single source of truth for the IPC contract with the Rust
 * backend. These mirror the `#[serde(rename_all = "camelCase")]` structs in
 * `src-tauri/src/models.rs` and `src-tauri/src/error.rs` exactly.
 *
 * IMPORTANT: only file *paths* and small metadata ever cross this boundary —
 * never PDF bytes. Large files are processed entirely by the native engine.
 */

/** Structured, user-facing error returned by every command (Rust `AppError`). */
export interface AppError {
  title: string;
  message: string;
  details?: string | null;
  suggestion?: string | null;
  code: string;
}

/** Metadata about a file on disk (Rust `FileInfo`). */
export interface FileInfo {
  path: string;
  name: string;
  sizeBytes: number;
  pageCount?: number | null;
  isValidPdf: boolean;
}

/** A file loaded into the workspace, with a stable unique id (so the same file
 * can be added twice without key collisions). */
export interface WorkspaceFile extends FileInfo {
  uid: string;
}

/** One bookmark/outline entry (Rust `OutlineItem`), flattened with a depth. */
export interface OutlineItem {
  title: string;
  page: number | null;
  level: number;
}

/** Visual page-comparison result (Rust `DiffResult`). */
export interface DiffResult {
  dataUrl: string;
  changedPercent: number;
}

/** Free-disk-space check result (Rust `DiskSpaceInfo`). */
export interface DiskSpaceInfo {
  path: string;
  availableBytes: number;
  requiredBytes: number;
  sufficient: boolean;
}

/** Result of a finished job (Rust `JobResult`). */
export interface JobResult {
  jobId: string;
  outputPaths: string[];
  status: string;
}

/** Lifecycle states for a job. */
export type JobState =
  | "idle"
  | "preparing"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

/** Live progress event payload (Rust `JobUpdate`, event name `job:update`). */
export interface JobUpdate {
  jobId: string;
  state: JobState;
  step: string;
  /** `null`/`undefined` => render an indeterminate progress bar. */
  percent?: number | null;
  message?: string | null;
}

/** One source file + an ordered qpdf page spec (Rust `PageGroup`). */
export interface PageGroup {
  path: string;
  pages: string;
}

/** A single page of a single source file (Rust `PagePick`). */
export interface PagePick {
  path: string;
  page: number;
}

/** A rotation for specific output pages (Rust `RotateGroup`). */
export interface RotateGroup {
  angle: number;
  pages: string;
}

/** A reference to a single page of a single source file (frontend-only). */
export interface PageRef {
  key: string;
  path: string;
  page: number;
  fileName: string;
}

/** A rendered page preview (Rust `RenderedThumb`). `dataUrl` is a small PNG. */
export interface RenderedThumb {
  page: number;
  dataUrl: string;
}

/** Split mode tagged union (Rust `SplitMode`). */
export type SplitMode =
  | { type: "everyN"; n: number }
  | { type: "ranges"; ranges: { start: number; end: number }[] };

/** Rotation angle accepted by the rotate operation. */
export type RotationAngle = 90 | 180 | 270;

/** A tool identifier used for routing and recent-jobs metadata. */
export type ToolId =
  | "merge"
  | "split"
  | "delete"
  | "extract"
  | "rotate"
  | "reorder"
  | "optimize"
  | "compress"
  | "images"
  | "repair"
  | "protect"
  | "officeToPdf"
  | "pdfToOffice"
  | "ocr"
  | "pageNumbers"
  | "unlock"
  | "watermark"
  | "crop"
  | "pdfa"
  | "compare"
  | "stamp"
  | "poster";

/** Locally-stored metadata about a completed/failed job (no PDF content). */
export interface RecentJob {
  id: string;
  tool: ToolId;
  /** Display label, e.g. "Merge 4 files". */
  label: string;
  status: Extract<JobState, "completed" | "failed" | "cancelled">;
  /** Epoch milliseconds. */
  finishedAt: number;
  outputPaths: string[];
  /** Short error summary if the job failed. */
  error?: string;
}

/** Type guard: is this value an AppError coming back from `invoke`? */
export function isAppError(value: unknown): value is AppError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "title" in value &&
    "message" in value
  );
}

/** Coerce anything thrown from a command into an AppError for the UI. */
export function toAppError(value: unknown): AppError {
  if (isAppError(value)) return value;
  if (value instanceof Error) {
    return {
      code: "UNKNOWN",
      title: "Something went wrong",
      message: value.message || "An unexpected error occurred.",
      details: value.stack ?? null,
    };
  }
  return {
    code: "UNKNOWN",
    title: "Something went wrong",
    message: typeof value === "string" ? value : "An unexpected error occurred.",
    details: (() => {
      try {
        return JSON.stringify(value);
      } catch {
        return null;
      }
    })(),
  };
}

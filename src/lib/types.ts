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

/** One supported `/Link` from `list_pdf_links` (unrotated `[x,y,w,h]` as a box). */
export type PdfLinkAction =
  | { type: "uri"; uri: string }
  | { type: "goto"; destPageIndex: number };

export interface PdfLink {
  pageIndex: number;
  rect: { x: number; y: number; w: number; h: number };
  action: PdfLinkAction;
}

/** One leftover or session annot listed from a PDF (Rust `ListedMarkup`). */
export interface ListedMarkup {
  pageIndex: number;
  subtype: string;
  rect: [number, number, number, number];
  author?: string | null;
  color?: number[] | null;
  contents?: string | null;
  quadPoints?: number[] | null;
  sessionId?: string | null;
}

/** Bounded editor image preview (Rust `ImagePreview`). */
export interface ImagePreview {
  dataUrl: string;
  width: number;
  height: number;
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
  | "editPdf"
  | "poster"
  | "nup"
  | "blankPages"
  | "metadata"
  | "textExport";

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

// ---------------------------------------------------------------------------
// Edit text (v0.4): mirrors of `src-tauri/src/pdf_engine/text_edit/dto.rs`.
// The code unions are cross-locked with `src/lib/editor/text-reasons.json`
// by `sourceTextCopy.test.ts` (and with the Rust enums by `reasons.rs`).
// ---------------------------------------------------------------------------

/** Why one line (run) can't be changed (§A.10 run level, priority order). */
export type TextReason =
  | "NESTED_FORM" | "SPLIT_CONTENT" | "SHARED_CONTENT" | "INLINE_IMAGE" | "INVISIBLE_TEXT" | "TEXT_CLIP_MODE"
  | "ZERO_SIZE" | "VERTICAL" | "MIRRORED_TEXT" | "ROTATED_TEXT" | "SKEWED_TEXT" | "CLIPPED" | "OPTIONAL_CONTENT"
  | "SOFT_MASK" | "PATTERN" | "ACTUAL_TEXT" | "MISSING_FONT" | "TYPE3" | "FONT_UNSUPPORTED" | "FONT_NOT_EMBEDDED"
  | "FONT_PROGRAM_UNSUPPORTED" | "FONT_PROGRAM_UNREADABLE" | "UNSUPPORTED_ENCODING" | "MISSING_WIDTHS"
  | "NO_TOUNICODE" | "AMBIGUOUS_UNICODE" | "RIGHT_TO_LEFT" | "COMPLEX_SCRIPT" | "DUPLICATE_TEXT"
  | "PER_GLYPH_TEXT" | "NO_WRITABLE_GLYPHS";
/** Why nothing on a page can be changed (every run is refused, none listed). */
export type TextPageReason = "MALFORMED_CONTENT" | "UNSUPPORTED_FILTER" | "PAGE_TOO_COMPLEX" | "GEOMETRY";
/** Why one edit can't be applied (preview verdicts; `AppError.code` at Save). */
export type TextEditProblemCode =
  | "GLYPH_MISSING" | "SPACE_NOT_WRITABLE" | "INVALID_TEXT" | "TEXT_TOO_LONG" | "TEXT_OUTSIDE_VISIBLE_AREA"
  | "FACE_UNAVAILABLE" | "STYLE_UNAVAILABLE" | "EDIT_CONFLICT" | "TEXT_EDIT_REFUSED" | "STALE"
  | "PEN_DRIFT" | "STATE_CHANGED" | "EDIT_VERIFY_FAILED";
/** Non-blocking notes about an edit. */
export type TextWarningCode = "NEXT_TEXT_OVERLAP" | "EDIT_NOT_VISIBLE" | "PREVIEW_UNAVAILABLE";
export type TextFace = "regular" | "bold" | "italic" | "boldItalic";
export type FamilyHint = "serif" | "sans" | "mono";

/** `open_text_source` result: the snapshot fingerprint every later call must quote. */
export interface TextSourceInfo { fingerprint: string; pageCount: number; warnings: string[] }
export interface TextVec { x: number; y: number }
/** Box in unrotated PDF user space (points), same space as `EditObject.rect`. */
export interface TextRect { x: number; y: number; w: number; h: number }
export interface TextRunMetrics {
  /** Font keys of the typing surface, primary first. */
  surface: string[];
  /** The `Tf` operand. */
  tfSize: number;
  /** What a reader sees, in points. */
  effectiveSize: number;
  /** `Tc` and `Tw`, unscaled text-space units. */
  charSpacing: number;
  wordSpacing: number;
  /** `Tz / 100`. */
  hScale: number;
  /** User units per unscaled text-space unit along the baseline (before `hScale`). */
  textToUser: number;
  /** Current letter spacing in effective points. */
  letterSpacingPt: number;
  spaceMode: "glyph" | "kern";
  /** TJ number (thousandths, negative) a typed space becomes in kern mode. */
  kernSpace: number;
  /** Ink extent along `dir` from `origin`, points. */
  originalWidth: number;
  /** Distance from `origin` to the edge of (visible box ∩ clip) along `dir`. */
  visibleExtent: number;
  /** Distance to the next run on the same line, or null. */
  nextObstacle: number | null;
}
export interface TextFaceOption { available: boolean; surface: string[] }
export interface TextRunStyle {
  /** `#rrggbb` when readable. */
  fill: string | null;
  sizeChangeable: boolean;
  colourChangeable: boolean;
  face: TextFace;
  faces: { regular: TextFaceOption; bold: TextFaceOption; italic: TextFaceOption; boldItalic: TextFaceOption };
}
export interface TextRun {
  id: string;
  /** Reading order (display space) and line cluster number. */
  order: number;
  line: number;
  text: string;
  rect: TextRect;
  origin: TextVec;
  /** Unit baseline direction in user space. */
  dir: TextVec;
  ascent: number;
  descent: number;
  /** `chars + 1` distances along `dir` from `origin` (user units). */
  caretOffsets: number[];
  editable: boolean;
  reason: TextReason | null;
  /** Present iff editable. */
  metrics: TextRunMetrics | null;
  style: TextRunStyle | null;
  substituted: boolean;
}
export interface TextFont {
  /** Opaque per response. */
  key: string;
  displayName: string;
  familyHint: FamilyHint;
  embedded: boolean;
  subset: boolean;
  /** Every typeable character of this font. */
  alphabet: string;
  /** `widths[i]` = glyph-space width (thousandths of text space, as `/Widths`) of the i-th character of `alphabet`. */
  widths: number[];
  /** The typeable space is the single byte 0x20, so `Tw` applies to it. */
  wordSpace: boolean;
}
export interface PageText {
  fingerprint: string;
  /** 0-based page in the source file. */
  pageIndex: number;
  pageReason: TextPageReason | null;
  runs: TextRun[];
  fonts: TextFont[];
}
/** Only fields that differ from the run's current style. */
export interface SourceTextStyle { sizePt?: number; face?: TextFace; fill?: string; letterSpacingPt?: number }
export interface TextEditIn { runId: string; originalText: string; text: string; style: SourceTextStyle }
export type TextStyleField = "size" | "face" | "colour";
export interface TextEditVerdict {
  runId: string;
  ok: boolean;
  code: TextEditProblemCode | null;
  chars: string[];
  reason: TextReason | null;
  face: TextFace | null;
  field: TextStyleField | null;
  detail: string | null;
  deltaPt: number;
  newRect: TextRect | null;
  /** Caret offsets of the new text (planned advances) when ok. */
  caretOffsets: number[] | null;
}
export interface TextWarning { runId: string; code: TextWarningCode; detail: string | null }
export interface TextPreview {
  /** Base64 one-page PDF, or null when the page can't be rendered here. */
  pagePdf: string | null;
  verdicts: TextEditVerdict[];
  pageProblem: { code: TextEditProblemCode; detail: string | null } | null;
  warnings: TextWarning[];
}

/** Type guard: is this value an AppError coming back from `invoke`? */
export function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  if (
    typeof candidate.code !== "string" ||
    typeof candidate.title !== "string" ||
    typeof candidate.message !== "string"
  ) {
    return false;
  }
  if (
    candidate.details !== undefined &&
    candidate.details !== null &&
    typeof candidate.details !== "string"
  ) {
    return false;
  }
  if (
    candidate.suggestion !== undefined &&
    candidate.suggestion !== null &&
    typeof candidate.suggestion !== "string"
  ) {
    return false;
  }
  return true;
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

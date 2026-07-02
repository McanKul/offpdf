/**
 * Typed wrappers around every Tauri command and the `job:update` event.
 *
 * This is the ONLY module in the frontend that talks to the backend. Errors are
 * normalized to `AppError` so callers always `catch (e) { toAppError(e) }`.
 *
 * Tauri maps camelCase JS argument keys to the snake_case Rust parameters, so
 * we pass `{ jobId, inputPaths }` for Rust `job_id`, `input_paths`.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DiskSpaceInfo,
  FileInfo,
  DiffResult,
  JobResult,
  JobUpdate,
  OutlineItem,
  PageGroup,
  PagePick,
  RenderedThumb,
  RotateGroup,
  RotationAngle,
  SplitMode,
} from "./types";

/** Generate a unique job id on the frontend (passed into every operation). */
export function newJobId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  // Fallback (should not be needed in a Tauri webview).
  return `job-${Date.now()}-${Math.floor(Math.random() * 1e9).toString(16)}`;
}

// ---------------------------------------------------------------------------
// File / system commands
// ---------------------------------------------------------------------------

export function pickPdfFiles(): Promise<string[]> {
  return invoke<string[]>("pick_pdf_files");
}

export function pickPdfFile(): Promise<string | null> {
  return invoke<string | null>("pick_pdf_file");
}

export function pickOutputFolder(): Promise<string | null> {
  return invoke<string | null>("pick_output_folder");
}

export function getFileInfo(path: string): Promise<FileInfo> {
  return invoke<FileInfo>("get_file_info", { path });
}

export function checkDiskSpace(
  path: string,
  requiredBytes: number,
): Promise<DiskSpaceInfo> {
  return invoke<DiskSpaceInfo>("check_disk_space", { path, requiredBytes });
}

export function openPath(path: string): Promise<void> {
  return invoke<void>("open_path", { path });
}

/** Clears all temp/job folders; returns bytes freed. */
export function clearTempFiles(): Promise<number> {
  return invoke<number>("clear_temp_files");
}

export function getTempDir(): Promise<string> {
  return invoke<string>("get_temp_dir");
}

/** Copy a file to a new path; returns the destination path. */
export function copyFile(src: string, dst: string): Promise<string> {
  return invoke<string>("copy_file", { src, dst });
}

// ---------------------------------------------------------------------------
// PDF operations — each takes a frontend-generated jobId and emits job:update.
// ---------------------------------------------------------------------------

export function mergePdfs(
  jobId: string,
  inputPaths: string[],
  outputPath: string,
): Promise<JobResult> {
  return invoke<JobResult>("merge_pdfs", { jobId, inputPaths, outputPath });
}

/** Assemble pages picked from one or more files, in order, into one PDF
 * (cross-document reorder / delete / extract). */
export function assemblePdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
): Promise<JobResult> {
  return invoke<JobResult>("assemble_pdf", { jobId, outputPath, groups });
}

export function splitPdf(
  jobId: string,
  outputDir: string,
  picks: PagePick[],
  mode: SplitMode,
): Promise<JobResult> {
  return invoke<JobResult>("split_pdf", { jobId, outputDir, picks, mode });
}

/** Organize: assemble kept pages (in order) and apply per-page rotations. */
export function editPdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  rotations: RotateGroup[],
): Promise<JobResult> {
  return invoke<JobResult>("edit_pdf", { jobId, outputPath, groups, rotations });
}

export function extractPages(
  jobId: string,
  inputPath: string,
  outputPath: string,
  pages: string,
): Promise<JobResult> {
  return invoke<JobResult>("extract_pages", {
    jobId,
    inputPath,
    outputPath,
    pages,
  });
}

export function deletePages(
  jobId: string,
  inputPath: string,
  outputPath: string,
  pages: string,
): Promise<JobResult> {
  return invoke<JobResult>("delete_pages", {
    jobId,
    inputPath,
    outputPath,
    pages,
  });
}

export function rotatePages(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  angle: RotationAngle,
  rotatePages: string,
): Promise<JobResult> {
  return invoke<JobResult>("rotate_pages", {
    jobId,
    outputPath,
    groups,
    angle,
    rotatePages,
  });
}

export function reorderPages(
  jobId: string,
  inputPath: string,
  outputPath: string,
  order: string,
): Promise<JobResult> {
  return invoke<JobResult>("reorder_pages", {
    jobId,
    inputPath,
    outputPath,
    order,
  });
}

export function optimizePdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
): Promise<JobResult> {
  return invoke<JobResult>("optimize_pdf", { jobId, outputPath, groups });
}

/**
 * Reduce file size by rasterizing each page to a JPEG at the given DPI/quality
 * and rewrapping into a PDF. Lossy (text becomes image) — best for scans and
 * large image-heavy plans.
 */
export function compressPdf(
  jobId: string,
  outputPath: string,
  picks: PagePick[],
  dpi: number,
  quality: number,
  targetBytes?: number,
): Promise<JobResult> {
  return invoke<JobResult>("compress_pdf", {
    jobId,
    outputPath,
    picks,
    dpi,
    quality,
    targetBytes: targetBytes ?? null,
  });
}

// ---------------------------------------------------------------------------
// Page preview / review
// ---------------------------------------------------------------------------

/** Whether a local page renderer (poppler) is available. */
export function rendererAvailable(): Promise<boolean> {
  return invoke<boolean>("renderer_available");
}

/** Convert an uploaded image into a one-page PDF; returns the new PDF path. */
export function imageToPdf(imagePath: string): Promise<string> {
  return invoke<string>("image_to_pdf", { imagePath });
}

/** Whether LibreOffice is available (for Office conversions). */
export function officeAvailable(): Promise<boolean> {
  return invoke<boolean>("office_available");
}

/** Per-page text of a PDF, for in-app search (only text crosses IPC). */
export function pdfText(inputPath: string): Promise<string[]> {
  return invoke<string[]>("pdf_text", { inputPath });
}

/** One page as a standalone PDF (base64) for true-vector zoom; null if too big. */
export function pagePdf(inputPath: string, page: number): Promise<string | null> {
  return invoke<string | null>("page_pdf", { inputPath, page });
}

/** A PDF's bookmarks/outline (flattened); empty if none or file too large. */
export function pdfOutline(inputPath: string): Promise<OutlineItem[]> {
  return invoke<OutlineItem[]>("pdf_outline", { inputPath });
}

/** Visually compare two pages; returns a diff-overlay image + changed percent. */
export function diffPages(
  aPath: string,
  aPage: number,
  bPath: string,
  bPage: number,
  size: number,
): Promise<DiffResult> {
  return invoke<DiffResult>("diff_pages", { aPath, aPage, bPath, bPage, size });
}

/** Whether Tesseract (OCR) is available. */
export function ocrAvailable(): Promise<boolean> {
  return invoke<boolean>("ocr_available");
}

/** OCR the combined document into one searchable PDF. `lang` e.g. "eng" or "tur". */
export function ocrPdf(
  jobId: string,
  outputPath: string,
  picks: PagePick[],
  lang: string,
): Promise<JobResult> {
  return invoke<JobResult>("ocr_pdf", { jobId, outputPath, picks, lang });
}

/** Stamp a line of text (typed signature / "APPROVED" / date) on one page. */
export function stampPdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  page: number,
  anchor: string,
  text: string,
  color: [number, number, number],
  sizePct: number,
): Promise<JobResult> {
  return invoke<JobResult>("stamp_pdf", { jobId, outputPath, groups, page, anchor, text, color, sizePct });
}

/**
 * Tile one large page into a grid of printable sheets (e.g. an A1 plan onto A4).
 * `tileW`/`tileH`/`overlap` are in PostScript points (1 mm = 2.83465 pt).
 */
export function posterPdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  page: number,
  tileW: number,
  tileH: number,
  overlap: number,
  marks: boolean,
): Promise<JobResult> {
  return invoke<JobResult>("poster_pdf", { jobId, outputPath, groups, page, tileW, tileH, overlap, marks });
}

/** Stamp page numbers onto the combined document. Optional Bates format:
 * `prefix` + zero-`padWidth`-padded counter + optional date (dd.MM.yyyy). */
export function addPageNumbers(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  position: string,
  start: number,
  prefix?: string,
  padWidth?: number,
  withDate?: boolean,
): Promise<JobResult> {
  return invoke<JobResult>("add_page_numbers", {
    jobId,
    outputPath,
    groups,
    position,
    start,
    prefix: prefix ?? null,
    padWidth: padWidth ?? null,
    withDate: withDate ?? null,
  });
}

/**
 * Lay 2 or 4 pages per sheet, or impose a saddle-stitch booklet (print
 * double-sided, flip on the short edge, fold + staple).
 * `sheetW`/`sheetH` are the output sheet size in PostScript points.
 */
export function nupPdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  mode: "2up" | "4up" | "booklet",
  sheetW: number,
  sheetH: number,
): Promise<JobResult> {
  return invoke<JobResult>("nup_pdf", { jobId, outputPath, groups, mode, sheetW, sheetH });
}

/** Convert an Office document to a PDF (temp); returns the new PDF path. */
export function officeToPdf(inputPath: string): Promise<string> {
  return invoke<string>("office_to_pdf", { inputPath });
}

/** Convert Office documents to PDFs saved into `outputDir`. */
export function officeToPdfBatch(
  jobId: string,
  outputDir: string,
  inputPaths: string[],
): Promise<JobResult> {
  return invoke<JobResult>("office_to_pdf_batch", { jobId, outputDir, inputPaths });
}

/** Convert the combined document to an Office format (docx/pptx/xlsx). */
export function pdfToOffice(
  jobId: string,
  outputDir: string,
  groups: PageGroup[],
  format: "docx" | "pptx" | "xlsx",
): Promise<JobResult> {
  return invoke<JobResult>("pdf_to_office", { jobId, outputDir, groups, format });
}

/** Export each page of the combined document to an image file (png/jpg). */
export function pdfToImages(
  jobId: string,
  outputDir: string,
  picks: PagePick[],
  format: "png" | "jpg",
  dpi: number,
): Promise<JobResult> {
  return invoke<JobResult>("pdf_to_images", { jobId, outputDir, picks, format, dpi });
}

/** Remove a password from an encrypted PDF (needs the correct password). */
export function unlockPdf(
  jobId: string,
  inputPath: string,
  outputPath: string,
  password: string,
): Promise<JobResult> {
  return invoke<JobResult>("unlock_pdf", { jobId, inputPath, outputPath, password });
}

/** Stamp a diagonal text watermark on every page. */
export function watermarkPdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  text: string,
  opacity: number,
): Promise<JobResult> {
  return invoke<JobResult>("watermark_pdf", { jobId, outputPath, groups, text, opacity });
}

/** Crop every page by trimming left/top/right/bottom percent. */
export function cropPdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  left: number,
  top: number,
  right: number,
  bottom: number,
): Promise<JobResult> {
  return invoke<JobResult>("crop_pdf", { jobId, outputPath, groups, left, top, right, bottom });
}

/** Convert the combined document to PDF/A-2b (via LibreOffice). */
export function pdfaPdf(jobId: string, outputPath: string, groups: PageGroup[]): Promise<JobResult> {
  return invoke<JobResult>("pdfa_pdf", { jobId, outputPath, groups });
}

/** Password-protect (encrypt) the combined document. */
export function protectPdf(
  jobId: string,
  outputPath: string,
  groups: PageGroup[],
  userPassword: string,
  ownerPassword: string,
): Promise<JobResult> {
  return invoke<JobResult>("protect_pdf", { jobId, outputPath, groups, userPassword, ownerPassword });
}

// ---------------------------------------------------------------------------
// Utility tools: blank pages / metadata / text export
// ---------------------------------------------------------------------------

/** Sensitivity presets for blank-page detection. */
export type BlankSensitivity = "strict" | "normal" | "aggressive";

/** The /Info metadata fields of a PDF. `null`/empty = not set (read) or remove (write). */
export interface PdfMeta {
  title: string | null;
  author: string | null;
  subject: string | null;
  keywords: string | null;
  creator: string | null;
  producer: string | null;
}

/** Detect blank pages (1-based) of one PDF. Cancellable via `cancelJob(jobId)`. */
export function detectBlankPages(
  jobId: string,
  inputPath: string,
  sensitivity: BlankSensitivity,
): Promise<number[]> {
  return invoke<number[]>("detect_blank_pages", { jobId, inputPath, sensitivity });
}

/** Read a PDF's /Info metadata (missing entries are null). */
export function readPdfMeta(inputPath: string): Promise<PdfMeta> {
  return invoke<PdfMeta>("read_pdf_meta", { inputPath });
}

/** Write /Info metadata to a copy of the PDF; `clearAll` strips everything (sanitize). */
export function writePdfMeta(
  jobId: string,
  inputPath: string,
  outputPath: string,
  fields: PdfMeta,
  clearAll: boolean,
): Promise<JobResult> {
  return invoke<JobResult>("write_pdf_meta", { jobId, inputPath, outputPath, fields, clearAll });
}

/** Export a PDF's text (whole doc or a 1-based page range) to a UTF-8 .txt file. */
export function exportPdfText(
  jobId: string,
  inputPath: string,
  outputPath: string,
  firstPage: number | null,
  lastPage: number | null,
): Promise<JobResult> {
  return invoke<JobResult>("export_pdf_text", { jobId, inputPath, outputPath, firstPage, lastPage });
}

/**
 * Render small PNG thumbnails for the given 1-based pages. Returns base64 data
 * URLs — only tiny rendered images cross IPC, never the source PDF bytes.
 */
export function renderThumbnails(
  inputPath: string,
  pages: number[],
  size?: number,
): Promise<RenderedThumb[]> {
  return invoke<RenderedThumb[]>("render_thumbnails", { inputPath, pages, size });
}

// ---------------------------------------------------------------------------
// Job control + events
// ---------------------------------------------------------------------------

export function cancelJob(jobId: string): Promise<void> {
  return invoke<void>("cancel_job", { jobId });
}

/**
 * Subscribe to job progress updates. Returns an unlisten function.
 * Pass a `jobId` to receive only that job's updates.
 */
export async function onJobUpdate(
  handler: (update: JobUpdate) => void,
  jobId?: string,
): Promise<UnlistenFn> {
  return listen<JobUpdate>("job:update", (event) => {
    if (!jobId || event.payload.jobId === jobId) {
      handler(event.payload);
    }
  });
}

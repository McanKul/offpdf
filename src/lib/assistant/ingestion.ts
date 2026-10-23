/**
 * Bounded, deterministic PDF ingestion for the private local assistant.
 *
 * This module is pure: it performs no I/O, no network access and no model
 * calls. A runtime adapter supplies native text extraction and explicit OCR.
 * The Tauri adapter lives in `ingestionTauri.ts`.
 *
 * Invariants:
 * - Only the explicitly supplied source paths are read; a containing folder is
 *   never scanned or indexed.
 * - Native text is preferred. OCR runs only when the caller explicitly enables
 *   it and only for pages without usable native text.
 * - Page boundaries are preserved: a chunk never spans two pages.
 * - Every chunk carries a stable source id, display basename and 1-based page.
 * - Sources, pages, characters, chunks, file size and OCR work are bounded.
 * - Cancellation stops between steps and surfaces `IngestionCancelledError`;
 *   the backend removes OCR temporary files, including on cancellation.
 * - Encrypted, malformed, empty and mixed documents produce structured issues
 *   instead of a thrown error, unless the caller cancels.
 */

import { RETRIEVAL_LIMITS, type PageChunk } from "./retrieval";

export interface IngestionLimits {
  /** Maximum documents considered in one call. Defaults to 20. */
  readonly maxSources: number;
  /** Maximum pages considered per source. Defaults to 1,000. */
  readonly maxPagesPerSource: number;
  /** Maximum pages considered across all sources. Defaults to 2,000. */
  readonly maxTotalPages: number;
  /** Maximum source file size. Defaults to 200 MiB. */
  readonly maxSourceBytes: number;
  /** Maximum total characters put into chunks. Defaults to 2,000,000. */
  readonly maxIndexChars: number;
  /** Maximum characters per chunk. Defaults to 4,000; capped at 16,000. */
  readonly maxChunkChars: number;
  /** Maximum chunks returned. Defaults to 5,000. */
  readonly maxChunks: number;
  /** Maximum OCR pages across the call. Defaults to 40; hard-capped at 200. */
  readonly maxOcrPages: number;
}

export const INGESTION_LIMITS: IngestionLimits = {
  maxSources: 20,
  maxPagesPerSource: 1_000,
  maxTotalPages: 2_000,
  maxSourceBytes: 200 * 1024 * 1024,
  maxIndexChars: RETRIEVAL_LIMITS.maxIndexChars,
  maxChunkChars: 4_000,
  maxChunks: RETRIEVAL_LIMITS.maxChunks,
  maxOcrPages: 40,
};

/** A file the user explicitly added to the assistant session. */
export interface IngestionSource {
  readonly path: string;
  /** Stable within the session (e.g. the workspace file uid). Defaults to f0, f1... */
  readonly id?: string;
  /** Optional display override; defaults to the file's basename. */
  readonly name?: string;
}

export interface IngestionFileInfo {
  readonly sizeBytes: number;
  readonly pageCount: number | null;
  readonly isValidPdf: boolean;
}

/** Bounded per-page text from the native extractor or one OCR pass. */
export interface ExtractedText {
  readonly pages: readonly string[];
  /** True when the backend hit its byte cap. */
  readonly truncated: boolean;
}

export interface IngestionRuntime {
  inspect(path: string): Promise<IngestionFileInfo>;
  extractText(path: string): Promise<ExtractedText>;
  /**
   * OCR only these 1-based pages. The backend must create, read and remove its
   * temporary searchable PDF, even when `signal` aborts the job.
   */
  ocrText(input: {
    readonly path: string;
    readonly pages: readonly number[];
    readonly language: string;
    readonly signal: AbortSignal;
    readonly onProgress?: (percent: number) => void;
  }): Promise<ExtractedText>;
}

export type IngestionIssueCode =
  | "NO_SOURCES"
  | "INVALID_SOURCE_ID"
  | "DUPLICATE_SOURCE_ID"
  | "TOO_MANY_SOURCES"
  | "NOT_PDF"
  | "UNREADABLE_PDF"
  | "EMPTY_PDF"
  | "SOURCE_TOO_LARGE"
  | "PAGE_LIMIT"
  | "EXTRACTION_FAILED"
  | "TEXT_LIMIT"
  | "OCR_REQUIRED"
  | "OCR_LANGUAGE_REQUIRED"
  | "OCR_LIMIT"
  | "OCR_EMPTY"
  | "OCR_UNAVAILABLE"
  | "OCR_FAILED"
  | "INDEX_LIMIT";

/** A structured, user-facing ingestion warning. Issues never contain file paths. */
export interface IngestionIssue {
  readonly sourceId?: string;
  readonly sourceName?: string;
  readonly code: IngestionIssueCode;
  readonly message: string;
  /** 1-based pages affected by this issue, when page-specific. */
  readonly pages?: readonly number[];
}

export interface IngestedSource {
  readonly id: string;
  /** Internal path used by the session UI to open a citation. */
  readonly path: string;
  /** Display basename only; never a directory. */
  readonly name: string;
  /** Page count reported by the backend (may exceed the pages considered). */
  readonly pageCount: number;
  readonly consideredPages: number;
  readonly nativePages: number;
  readonly ocrPages: number;
  readonly emptyPages: number;
}

export interface IngestionStats {
  readonly sourceCount: number;
  readonly pageCount: number;
  readonly nativePages: number;
  readonly ocrPages: number;
  readonly emptyPages: number;
  readonly chunkCount: number;
  readonly chars: number;
  /** True when any bound omitted pages, text or OCR work. */
  readonly truncated: boolean;
}

export interface IngestionResult {
  readonly chunks: PageChunk[];
  readonly sources: IngestedSource[];
  readonly issues: IngestionIssue[];
  readonly stats: IngestionStats;
}

export type IngestionStage = "inspecting" | "extracting" | "ocr" | "chunking" | "done";

export interface IngestionProgress {
  readonly sourceId: string;
  readonly sourceIndex: number;
  readonly sourceCount: number;
  readonly stage: IngestionStage;
  /** Overall progress across the call, 0-100. */
  readonly percent: number;
}

export interface IngestionOcr {
  /** OCR is never implicit; the UI must ask the user first. */
  readonly enabled: boolean;
  /** Installed Tesseract language code, e.g. "eng" or "eng+tur". */
  readonly language?: string;
  /**
   * Pages to OCR, or "auto" (default) for every page without usable native
   * text. Pages with usable native text are never replaced by OCR.
   */
  readonly pages?: readonly number[] | "auto";
  /** Per-call OCR page cap. Defaults to 40 and is capped at 200. */
  readonly maxPages?: number;
}

export interface IngestionOptions {
  readonly ocr?: IngestionOcr;
  readonly limits?: Partial<IngestionLimits>;
  readonly signal?: AbortSignal;
  readonly onProgress?: (progress: IngestionProgress) => void;
}

export class IngestionCancelledError extends Error {
  readonly code = "INGESTION_CANCELLED";

  constructor() {
    super("Document ingestion was cancelled.");
    this.name = "IngestionCancelledError";
  }
}

export function isIngestionCancelled(
  error: unknown,
): error is IngestionCancelledError {
  if (error instanceof IngestionCancelledError) return true;
  return (
    typeof error === "object" &&
    error !== null &&
    ((error as { code?: unknown }).code === "INGESTION_CANCELLED" ||
      (error as { code?: unknown }).code === "CANCELLED" ||
      (error as { name?: unknown }).name === "AbortError")
  );
}

const SOURCE_ID_RE = /^[A-Za-z0-9._-]{1,64}$/;
const CONTROL_CHARS_RE = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g;
const MAX_CHUNK_ID_CHARS = 128;
const MIN_USABLE_TEXT_CHARS = 8;
const HARD_MAX_OCR_PAGES = 200;

/**
 * Read the explicitly supplied PDFs, extract native text, optionally OCR pages
 * without usable text, and return bounded page-aligned chunks.
 *
 * Extraction and OCR failures are reported as issues; the call resolves with
 * whatever could be ingested. User cancellation rejects with
 * `IngestionCancelledError`.
 */
export async function ingestPdfSources(
  inputSources: readonly IngestionSource[],
  runtime: IngestionRuntime,
  options: IngestionOptions = {},
): Promise<IngestionResult> {
  const limits = resolveLimits(options.limits);
  const signal = options.signal ?? new AbortController().signal;
  throwIfAborted(signal);

  const issues: IngestionIssue[] = [];
  const chunks: PageChunk[] = [];
  const sources: IngestedSource[] = [];
  const seenIds = new Set<string>();
  let totalPages = 0;
  let totalChars = 0;
  let nativePages = 0;
  let ocrPages = 0;
  let ocrPagesUsed = 0;
  let emptyPages = 0;
  let truncated = false;
  let indexLimitReported = false;

  const selected = inputSources.slice(0, limits.maxSources);
  if (inputSources.length === 0) {
    issues.push({
      code: "NO_SOURCES",
      message: "Select at least one PDF before asking a question.",
    });
  } else if (inputSources.length > selected.length) {
    truncated = true;
    issues.push({
      code: "TOO_MANY_SOURCES",
      message: `Only the first ${limits.maxSources} documents can be ingested at once.`,
    });
  }

  for (let sourceIndex = 0; sourceIndex < selected.length; sourceIndex++) {
    throwIfAborted(signal);
    const source = selected[sourceIndex];
    const id = normalizeSourceId(source.id, sourceIndex);
    const name = sourceBasename(source.name ?? source.path);

    if (!id) {
      issues.push({
        sourceName: name,
        code: "INVALID_SOURCE_ID",
        message: "This document has an invalid assistant source id.",
      });
      continue;
    }
    if (seenIds.has(id)) {
      issues.push({
        sourceId: id,
        sourceName: name,
        code: "DUPLICATE_SOURCE_ID",
        message: "Two documents use the same assistant source id.",
      });
      continue;
    }
    seenIds.add(id);

    report(options, {
      sourceId: id,
      sourceIndex,
      sourceCount: selected.length,
      stage: "inspecting",
      percent: (sourceIndex / selected.length) * 100,
    });

    let info: IngestionFileInfo;
    try {
      info = await runtime.inspect(source.path);
    } catch (error) {
      if (signal.aborted || isIngestionCancelled(error)) {
        throw new IngestionCancelledError();
      }
      issues.push({
        sourceId: id,
        sourceName: name,
        code: "UNREADABLE_PDF",
        message: "The PDF could not be read. It may be encrypted or damaged.",
      });
      continue;
    }
    throwIfAborted(signal);

    if (!info.isValidPdf) {
      // get_file_info uses the native engine, so false covers non-PDFs,
      // encrypted files without a password, and malformed documents.
      issues.push({
        sourceId: id,
        sourceName: name,
        code: "NOT_PDF",
        message:
          "This file is not a readable PDF. It may be encrypted, damaged or not a PDF.",
      });
      continue;
    }
    if (info.sizeBytes > limits.maxSourceBytes) {
      truncated = true;
      issues.push({
        sourceId: id,
        sourceName: name,
        code: "SOURCE_TOO_LARGE",
        message: "This document is larger than the assistant ingestion limit.",
      });
      continue;
    }

    const pageCount = info.pageCount ?? 0;
    if (pageCount <= 0) {
      issues.push({
        sourceId: id,
        sourceName: name,
        code: "EMPTY_PDF",
        message: "This PDF has no pages.",
      });
      continue;
    }

    const remainingPages = Math.max(0, limits.maxTotalPages - totalPages);
    const considered = Math.min(
      pageCount,
      limits.maxPagesPerSource,
      remainingPages,
    );
    if (considered < pageCount) {
      truncated = true;
      issues.push({
        sourceId: id,
        sourceName: name,
        code: "PAGE_LIMIT",
        message: `Only the first ${considered} of ${pageCount} pages were considered.`,
      });
    }
    if (considered <= 0) {
      truncated = true;
      issues.push({
        code: "PAGE_LIMIT",
        message: "The assistant page budget is exhausted.",
      });
      break;
    }

    report(options, {
      sourceId: id,
      sourceIndex,
      sourceCount: selected.length,
      stage: "extracting",
      percent: ((sourceIndex + 0.25) / selected.length) * 100,
    });

    let extracted: ExtractedText;
    try {
      extracted = await runtime.extractText(source.path);
    } catch (error) {
      if (signal.aborted || isIngestionCancelled(error)) {
        throw new IngestionCancelledError();
      }
      issues.push({
        sourceId: id,
        sourceName: name,
        code: "EXTRACTION_FAILED",
        message: "Native text extraction failed for this document.",
      });
      continue;
    }
    throwIfAborted(signal);

    if (extracted.truncated) {
      truncated = true;
      issues.push({
        sourceId: id,
        sourceName: name,
        code: "TEXT_LIMIT",
        message: "The document text was capped before all pages were read.",
      });
    }

    const nativeTexts: string[] = [];
    const emptyPageNumbers: number[] = [];
    for (let page = 1; page <= considered; page++) {
      const text = normalizePageText(extracted.pages[page - 1] ?? "");
      nativeTexts.push(text);
      if (!hasUsableText(text)) emptyPageNumbers.push(page);
    }

    const ocrTexts = new Map<number, string>();
    if (emptyPageNumbers.length > 0) {
      const ocr = options.ocr;
      if (!ocr?.enabled) {
        issues.push({
          sourceId: id,
          sourceName: name,
          code: "OCR_REQUIRED",
          message:
            "Some pages have no usable text layer. OCR must be requested explicitly.",
          pages: emptyPageNumbers,
        });
      } else if (!ocr.language?.trim()) {
        issues.push({
          sourceId: id,
          sourceName: name,
          code: "OCR_LANGUAGE_REQUIRED",
          message: "Choose an installed OCR language before using OCR.",
          pages: emptyPageNumbers,
        });
      } else {
        const requested = selectOcrPages(
          ocr.pages,
          emptyPageNumbers,
          considered,
        );
        const maxOcr = Math.max(
          0,
          Math.min(
            positiveInt(ocr.maxPages, limits.maxOcrPages),
            limits.maxOcrPages - ocrPagesUsed,
            HARD_MAX_OCR_PAGES - ocrPagesUsed,
          ),
        );
        const ocrPageNumbers = requested.slice(0, maxOcr);
        if (ocrPageNumbers.length < requested.length) {
          truncated = true;
          issues.push({
            sourceId: id,
            sourceName: name,
            code: "OCR_LIMIT",
            message: `Only ${ocrPageNumbers.length} pages were OCR'd in this pass.`,
            pages: requested.slice(ocrPageNumbers.length),
          });
        }

        if (ocrPageNumbers.length > 0) {
          report(options, {
            sourceId: id,
            sourceIndex,
            sourceCount: selected.length,
            stage: "ocr",
            percent: ((sourceIndex + 0.4) / selected.length) * 100,
          });
          let ocrResult: ExtractedText | null = null;
          try {
            ocrResult = await runtime.ocrText({
              path: source.path,
              pages: ocrPageNumbers,
              language: ocr.language.trim(),
              signal,
              onProgress: (percent) => {
                const safePercent = Number.isFinite(percent)
                  ? Math.max(0, Math.min(100, percent))
                  : 0;
                report(options, {
                  sourceId: id,
                  sourceIndex,
                  sourceCount: selected.length,
                  stage: "ocr",
                  percent: Math.min(
                    100,
                    ((sourceIndex + 0.4 + (safePercent / 100) * 0.4) /
                      selected.length) *
                      100,
                  ),
                });
              },
            });
          } catch (error) {
            if (signal.aborted || isIngestionCancelled(error)) {
              throw new IngestionCancelledError();
            }
            issues.push({
              sourceId: id,
              sourceName: name,
              code: classifyOcrError(error),
              message: ocrErrorMessage(error),
              pages: ocrPageNumbers,
            });
          }
          ocrPagesUsed += ocrPageNumbers.length;
          throwIfAborted(signal);

          if (ocrResult) {
            if (ocrResult.truncated) {
              truncated = true;
              issues.push({
                sourceId: id,
                sourceName: name,
                code: "TEXT_LIMIT",
                message: "OCR text was capped before every page was read.",
              });
            }
            const stillEmpty: number[] = [];
            for (let i = 0; i < ocrPageNumbers.length; i++) {
              const page = ocrPageNumbers[i];
              const text = normalizePageText(ocrResult.pages[i] ?? "");
              if (hasUsableText(text)) {
                ocrTexts.set(page, text);
              } else {
                stillEmpty.push(page);
              }
            }
            if (stillEmpty.length > 0) {
              issues.push({
                sourceId: id,
                sourceName: name,
                code: "OCR_EMPTY",
                message: "OCR did not find usable text on some pages.",
                pages: stillEmpty,
              });
            }
          }
        }
      }
    }

    report(options, {
      sourceId: id,
      sourceIndex,
      sourceCount: selected.length,
      stage: "chunking",
      percent: ((sourceIndex + 0.8) / selected.length) * 100,
    });

    // Count page outcomes from the page classification, not from how far the
    // chunk loop got, so a mid-page index limit does not under-report pages.
    const sourceOcr = ocrTexts.size;
    const sourceEmpty = emptyPageNumbers.length - sourceOcr;
    const sourceNative = considered - emptyPageNumbers.length;
    let hitIndexLimit = false;
    for (let page = 1; page <= considered; page++) {
      throwIfAborted(signal);
      if (chunks.length >= limits.maxChunks || totalChars >= limits.maxIndexChars) {
        truncated = true;
        hitIndexLimit = true;
        if (!indexLimitReported) {
          indexLimitReported = true;
          issues.push({
            code: "INDEX_LIMIT",
            message: "The assistant context index reached its size limit.",
          });
        }
        break;
      }

      const ocrText = ocrTexts.get(page);
      const text = ocrText ?? nativeTexts[page - 1] ?? "";
      if (!hasUsableText(text)) continue;

      const pageChunks = splitPageText(text, limits.maxChunkChars);
      for (let chunkIndex = 0; chunkIndex < pageChunks.length; chunkIndex++) {
        const chunkText = pageChunks[chunkIndex];
        if (
          chunks.length >= limits.maxChunks ||
          totalChars + chunkText.length > limits.maxIndexChars
        ) {
          truncated = true;
          hitIndexLimit = true;
          if (!indexLimitReported) {
            indexLimitReported = true;
            issues.push({
              code: "INDEX_LIMIT",
              message: "The assistant context index reached its size limit.",
            });
          }
          break;
        }
        chunks.push({
          id: makeChunkId(id, page, chunkIndex + 1),
          sourceId: id,
          sourceName: name,
          page,
          text: chunkText,
        });
        totalChars += chunkText.length;
      }
      if (hitIndexLimit) break;
    }

    nativePages += sourceNative;
    ocrPages += sourceOcr;
    emptyPages += sourceEmpty;
    totalPages += considered;
    sources.push({
      id,
      path: source.path,
      name,
      pageCount,
      consideredPages: considered,
      nativePages: sourceNative,
      ocrPages: sourceOcr,
      emptyPages: sourceEmpty,
    });

    if (hitIndexLimit) break;
  }

  if (selected.length > 0) {
    report(options, {
      sourceId: sources[sources.length - 1]?.id ?? "",
      sourceIndex: selected.length,
      sourceCount: selected.length,
      stage: "done",
      percent: 100,
    });
  }

  return {
    chunks,
    sources,
    issues,
    stats: {
      sourceCount: sources.length,
      pageCount: totalPages,
      nativePages,
      ocrPages,
      emptyPages,
      chunkCount: chunks.length,
      chars: totalChars,
      truncated,
    },
  };
}

/** Normalize a page's text without merging it with any other page. */
export function normalizePageText(text: string): string {
  return text
    .replace(/\r\n?/g, "\n")
    .replace(/\u00a0/g, " ")
    .replace(CONTROL_CHARS_RE, "")
    .split("\n")
    .map((line) => line.replace(/[ \t]+/g, " ").trim())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** A page is usable when it has more than a stray page number or punctuation. */
export function hasUsableText(text: string): boolean {
  const normalized = normalizePageText(text);
  if (normalized.length < MIN_USABLE_TEXT_CHARS) return false;
  const words = normalized.match(/[\p{L}\p{N}]+/gu) ?? [];
  if (words.length >= 2) return true;
  return words.join("").length >= MIN_USABLE_TEXT_CHARS;
}

/**
 * Split one page into chunks no longer than `maxChars`. Paragraph and word
 * boundaries are preferred, but a single unbroken token is hard-split. The
 * returned chunks never include text from another page.
 */
export function splitPageText(text: string, maxChars: number): string[] {
  const normalized = normalizePageText(text);
  if (!normalized) return [];
  const limit = Math.max(
    1,
    Math.min(
      Number.isFinite(maxChars) ? Math.floor(maxChars) : 4_000,
      RETRIEVAL_LIMITS.maxChunkChars,
    ),
  );
  if (normalized.length <= limit) return [normalized];

  const paragraphs = normalized
    .split(/\n{2,}/)
    .map((paragraph) => paragraph.trim())
    .filter(Boolean);
  const chunks: string[] = [];
  let current = "";

  for (const paragraph of paragraphs) {
    for (const piece of splitLongText(paragraph, limit)) {
      if (!current) {
        current = piece;
      } else if (current.length + 2 + piece.length <= limit) {
        current = `${current}\n\n${piece}`;
      } else {
        chunks.push(current);
        current = piece;
      }
    }
  }
  if (current) chunks.push(current);
  return chunks;
}

/** Display basename only, with control characters and separators removed. */
export function sourceBasename(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const last = normalized.slice(normalized.lastIndexOf("/") + 1);
  const name = last
    .replace(CONTROL_CHARS_RE, "")
    .replace(/[\\/]/g, "_")
    .trim();
  return (name || "document.pdf").slice(
    0,
    RETRIEVAL_LIMITS.maxSourceNameChars,
  );
}

function resolveLimits(overrides: Partial<IngestionLimits> | undefined): IngestionLimits {
  return {
    maxSources: positiveInt(overrides?.maxSources, INGESTION_LIMITS.maxSources),
    maxPagesPerSource: positiveInt(
      overrides?.maxPagesPerSource,
      INGESTION_LIMITS.maxPagesPerSource,
    ),
    maxTotalPages: positiveInt(
      overrides?.maxTotalPages,
      INGESTION_LIMITS.maxTotalPages,
    ),
    maxSourceBytes: positiveInt(
      overrides?.maxSourceBytes,
      INGESTION_LIMITS.maxSourceBytes,
    ),
    maxIndexChars: Math.min(
      positiveInt(overrides?.maxIndexChars, INGESTION_LIMITS.maxIndexChars),
      RETRIEVAL_LIMITS.maxIndexChars,
    ),
    maxChunkChars: Math.min(
      Math.max(1, positiveInt(overrides?.maxChunkChars, INGESTION_LIMITS.maxChunkChars)),
      RETRIEVAL_LIMITS.maxChunkChars,
    ),
    maxChunks: Math.min(
      positiveInt(overrides?.maxChunks, INGESTION_LIMITS.maxChunks),
      RETRIEVAL_LIMITS.maxChunks,
    ),
    maxOcrPages: Math.min(
      positiveInt(overrides?.maxOcrPages, INGESTION_LIMITS.maxOcrPages),
      HARD_MAX_OCR_PAGES,
    ),
  };
}

function positiveInt(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? Math.max(1, Math.floor(value))
    : fallback;
}

function normalizeSourceId(
  value: string | undefined,
  sourceIndex: number,
): string | null {
  const candidate = value?.trim() || `f${sourceIndex}`;
  if (candidate === "." || candidate === "..") return null;
  return SOURCE_ID_RE.test(candidate) ? candidate : null;
}

function selectOcrPages(
  requested: readonly number[] | "auto" | undefined,
  emptyPageNumbers: readonly number[],
  consideredPages: number,
): number[] {
  if (requested === undefined || requested === "auto") {
    return [...emptyPageNumbers];
  }
  const empty = new Set(emptyPageNumbers);
  const seen = new Set<number>();
  const pages: number[] = [];
  for (const page of requested) {
    if (
      Number.isInteger(page) &&
      page >= 1 &&
      page <= consideredPages &&
      empty.has(page) &&
      !seen.has(page)
    ) {
      seen.add(page);
      pages.push(page);
    }
  }
  return pages;
}

function splitLongText(text: string, limit: number): string[] {
  const chunks: string[] = [];
  let rest = text.trim();
  while (rest.length > limit) {
    let cut = rest.lastIndexOf(" ", limit);
    if (cut < Math.floor(limit / 2)) {
      const newline = rest.lastIndexOf("\n", limit);
      if (newline > cut) cut = newline;
    }
    if (cut <= 0) cut = limit;
    const head = rest.slice(0, cut).trim();
    if (head) chunks.push(head);
    rest = rest.slice(cut).trim();
  }
  if (rest) chunks.push(rest);
  return chunks;
}

function makeChunkId(sourceId: string, page: number, chunkIndex: number): string {
  const id = `${sourceId}:p${page}:c${chunkIndex}`;
  return id.length <= MAX_CHUNK_ID_CHARS
    ? id
    : id.slice(0, MAX_CHUNK_ID_CHARS);
}

function classifyOcrError(error: unknown): IngestionIssueCode {
  const code =
    typeof error === "object" && error !== null
      ? (error as { code?: unknown }).code
      : undefined;
  if (
    code === "OCR_MISSING" ||
    code === "OCR_LANGS_FAILED" ||
    code === "OCR_LANG_EMPTY" ||
    code === "OCR_LANG_MISSING"
  ) {
    return "OCR_UNAVAILABLE";
  }
  return "OCR_FAILED";
}

function ocrErrorMessage(error: unknown): string {
  const code =
    typeof error === "object" && error !== null
      ? (error as { code?: unknown }).code
      : undefined;
  if (code === "OCR_MISSING") {
    return "OCR needs Tesseract, which was not found on this machine.";
  }
  if (code === "OCR_LANG_MISSING" || code === "OCR_LANG_EMPTY") {
    return "The selected OCR language pack is not installed.";
  }
  // Do not surface backend details here: they can contain the source path,
  // while issues intentionally carry only the display basename.
  return "OCR could not run for this document.";
}

function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) throw new IngestionCancelledError();
}

function report(
  options: IngestionOptions,
  progress: IngestionProgress,
): void {
  if (!options.onProgress) return;
  try {
    options.onProgress(progress);
  } catch {
    // A broken progress listener must never break ingestion.
  }
}

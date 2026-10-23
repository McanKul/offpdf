# Assistant ingestion

Ingestion turns the PDFs a user explicitly selected into bounded `PageChunk`
values for the private local assistant. It is deterministic, offline and
session-only: there is no background watcher and no permanent index.

The implementation lives in:

- `src/lib/assistant/ingestion.ts` — pure orchestration, bounds and chunking.
- `src/lib/assistant/ingestionTauri.ts` — Tauri runtime adapter.
- `src-tauri/src/commands/assistant.rs` — bounded native text and explicit OCR.

## What is read

Only the paths in the source list are read. A containing folder is never
scanned, watched or indexed. The source list is explicit:

```ts
interface IngestionSource {
  path: string;   // the selected file
  id?: string;    // stable session id, e.g. the workspace uid
  name?: string;  // optional display name; defaults to the basename
}
```

`PageChunk` values contain `sourceId`, the display basename and a 1-based page,
but never a filesystem path. The session keeps the `path` in its own
`IngestedSource` map so it can open a cited page.

## Native text first

Native extraction is preferred:

1. `assistantPdfText` calls `pdftotext` with an 8 MiB byte cap.
2. Text is normalized page by page. Empty pages stay empty; they are never
   merged with a neighbouring page.
3. A page with only a page number, punctuation or whitespace is treated as
   having no usable text layer.

OCR is never implicit. When a page has no usable text and OCR was not requested,
ingestion returns an `OCR_REQUIRED` issue with the affected 1-based pages. The
UI can then ask the user whether to run OCR.

## Explicit OCR

OCR is enabled by the caller:

```ts
const result = await ingestPdfSources(sources, runtime, {
  ocr: {
    enabled: true,
    language: "eng",      // installed Tesseract language code
    pages: "auto",         // default: pages without usable native text
    maxPages: 40,
  },
  signal,
});
```

Rules:

- OCR only runs on pages without usable native text. A page with native text is
  never replaced by OCR, even if it was listed explicitly.
- `pages: "auto"` (the default) selects every unusable page. An explicit array
  selects a subset; pages with native text in that array are ignored.
- Mixed documents work page by page: native pages stay native, scanned pages
  are OCR'd.
- If OCR is enabled without a language, ingestion returns
  `OCR_LANGUAGE_REQUIRED` and does not start Tesseract.
- OCR runs only for the requested documents and pages, offline, using the local
  Tesseract installation.

The backend command `assistant_ocr_text` builds one temporary searchable PDF,
extracts its per-page text, and always removes that PDF before returning. The
temporary file is also removed when OCR fails or the job is cancelled. The
per-page work directory used by the OCR engine is removed as well.

## Chunking and page identity

Each page is chunked independently, so a chunk never spans two pages. Chunks
prefer paragraph and word boundaries and are hard-split only when a single token
is longer than the limit.

```ts
interface PageChunk {
  id: string;          // e.g. "f1:p4:c2"
  sourceId: string;    // stable in the session
  sourceName: string;  // basename only
  page: number;        // 1-based
  text: string;
}
```

Chunk ids are deterministic for the same input and limits. Empty pages produce
no chunk.

## Limits

Defaults are deliberately smaller than the retrieval maximum because the local
model only needs a bounded context.

| Limit | Default |
| ----- | ------- |
| Sources per call | 20 |
| Pages per source | 1,000 |
| Pages across the call | 2,000 |
| Source file size | 200 MiB |
| Indexed characters | 2,000,000 |
| Chunk size | 4,000 characters |
| Chunks per call | 5,000 |
| OCR pages per call | 40 (backend hard cap: 200) |
| Native text per source/OCR pass | 8 MiB |

When a limit omits work, ingestion keeps the safe portion and sets
`stats.truncated` plus an issue such as `PAGE_LIMIT`, `OCR_LIMIT`,
`TEXT_LIMIT`, `INDEX_LIMIT`, `SOURCE_TOO_LARGE` or `TOO_MANY_SOURCES`.

## Failure and edge-case behavior

| Document | Behavior |
| -------- | -------- |
| Native text | Native chunks; no OCR. |
| Scanned / image-only | No chunks unless OCR was explicitly enabled; `OCR_REQUIRED` otherwise. |
| Mixed | Native pages keep native text; unusable pages can be OCR'd. |
| Empty PDF | `EMPTY_PDF`, no chunks. |
| Malformed or unreadable PDF | `NOT_PDF` / `UNREADABLE_PDF`, no chunks. |
| Encrypted PDF without a password | Never guessed or bypassed. If metadata fails, it is reported as `NOT_PDF` / `UNREADABLE_PDF` and is not indexed. If the engine can read the page tree but not the text, the pages are treated as having no usable text; OCR, if requested, fails with a structured issue. The user must unlock it first. |
| OCR unavailable | `OCR_UNAVAILABLE`; native chunks, if any, are still returned. |
| OCR finds no text | `OCR_EMPTY`; the page remains empty. |
| Extraction/OCR command failure | A structured issue; other sources and native chunks are not discarded. |

Issues carry only the source id and display basename, not filesystem paths.

## Cancellation

Callers pass an `AbortSignal`. Ingestion checks it between steps and exposes
`IngestionCancelledError` (or `isIngestionCancelled(error)`). During OCR the
adapter calls `cancelJob`, which kills the Tesseract/poppler child. The native
command removes its temporary searchable PDF before returning, so cancellation
does not leave ingestion data behind.

No partial result is returned on cancellation; the caller should discard the
session's chunks and retry if needed.

## API

```ts
import { ingestPdfSources } from "@/lib/assistant/ingestion";
import { createTauriIngestionRuntime } from "@/lib/assistant/ingestionTauri";

const controller = new AbortController();
const result = await ingestPdfSources(
  [{ id: "f1", path: selectedPath }],
  createTauriIngestionRuntime(),
  {
    ocr: { enabled: true, language: "eng" },
    signal: controller.signal,
    onProgress: (progress) => {
      // progress.stage: inspecting | extracting | ocr | chunking | done
      // progress.percent: 0-100 overall
    },
  },
);

result.chunks;  // PageChunk[]
result.sources; // id, path, name, pageCount, nativePages, ocrPages...
result.issues;  // structured, path-free warnings
result.stats;   // counts and truncated
```

`ingestPdfSources` is pure with respect to the runtime: tests inject a fake
runtime and never need Tesseract or a real PDF. The production adapter is the
only part that talks to Tauri.

## Fixtures and tests

- `src/lib/assistant/fixtures/ingestion.json` — representative native, scanned
  and mixed pages, plus OCR output and a long page.
- `src/lib/assistant/ingestion.test.ts` — native/scanned/mixed coverage,
  page-boundary preservation, explicit OCR, language and page limits,
  malformed/encrypted/empty behavior, cancellation, folder isolation, source
  metadata and deterministic output.
- Rust unit tests cover the form-feed page splitter; backend compilation and the
  OCR command are checked by CI.

No ingestion or test path performs a network request.

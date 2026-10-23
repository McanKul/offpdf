# Assistant grounded answers and citations

The private local assistant answers questions from the PDF the user selected by
retrieving a bounded set of page chunks and validating every citation against
the text the model actually received.

This layer is intentionally narrow:

- Ingestion (#86) extracts native text, offers OCR, and creates chunks that keep
  a stable source id and a 1-based page number.
- This module retrieves chunks, validates the model's answer envelope, and
  formats citations. It never reads files, calls the network, or loads a model.
- The session UI (#88) maps a citation target to the existing page viewer.

The implementation lives in `src/lib/assistant/retrieval.ts`,
`src/lib/assistant/citations.ts`, and `src/lib/assistant/ingestion.ts` (with a
Tauri adapter in `ingestionTauri.ts`). See
[assistant-ingestion.md](assistant-ingestion.md) for extraction, OCR, bounds and
cancellation behavior.

## Pipeline

1. Ingestion supplies `PageChunk` values.
2. `retrieveChunks(query, chunks)` ranks a bounded context.
3. The local runtime asks the model for a JSON answer envelope.
4. `groundAnswerJson(query, chunks, json)` validates the envelope against the
   retrieved context and returns a `GroundedAnswer` or an honest failure.
5. The UI renders the answer and its citations; `citationTarget()` provides the
   source id and page to open.

A model cannot cite a chunk that was not retrieved, and it cannot quote text
that is not present in the cited chunk. If no chunk matches the question, the
assistant reports insufficient evidence instead of answering from general
knowledge.

## Page chunk contract

```ts
interface PageChunk {
  id: string;          // stable within the session; never a path
  sourceId: string;    // workspace/source id, e.g. "f1"
  sourceName: string;  // basename only, e.g. "annual-report.pdf"
  page: number;        // 1-based
  text: string;        // text for this chunk
}
```

Chunks may be split within a page, but the page metadata must survive chunking.
`sourceName` is display-only; path separators and drive prefixes are rejected.
The assistant never receives a filesystem path.

## Retrieval

`retrieveChunks` is deterministic and bounded:

- BM25 ranking over Unicode word terms.
- Document indexing preserves repeated terms for term-frequency scoring; query
  terms are deduplicated.
- Exact-phrase bonus when the normalized query occurs in a chunk.
- Stable tie-breaks by input order, then chunk id.
- Defaults: 6 chunks and 12,000 characters. Caps: 12 chunks and 24,000
  characters.
- Input caps: 5,000 chunks, 16,000 characters per chunk, and 2,000,000 indexed
  characters. Chunks with invalid metadata, duplicate ids, empty text, or
  oversized text are ignored and set `truncated`.
- `truncated` means the caller is not seeing every matching chunk; it does not
  mean the answer is invalid.

Tokenization uses `Intl.Segmenter` when available and falls back to Unicode word
runs. For scripts that do not reliably separate words with spaces (Han,
Hiragana, Katakana, Hangul, Thai, Lao, Khmer, Myanmar), terms also include
characters and bigrams. Matching folds case, combining marks, and a small set of
common OCR variants (`ı`, `ß`, `æ`, `œ`, `ø`, `ł`, `đ`, `ħ`). This keeps queries
such as `facturacion` useful against `facturación` and `istanbul` against
`İstanbul` without an English-only stop-word or stemming step.

## Answer envelope

The model returns one JSON object. Unknown fields are rejected.

```json
{
  "version": 1,
  "kind": "answer",
  "text": "Revenue was 4.2 million euros in 2025.",
  "citations": [
    {
      "chunkId": "c1",
      "quote": "Revenue was 4.2 million euros."
    }
  ]
}
```

| Field       | Required | Meaning |
| ----------- | -------- | ------- |
| `version`   | yes      | Exactly `1`. |
| `kind`      | yes      | `answer` for a question, `summary` for a document summary, or `insufficient` for an honest refusal. |
| `text`      | yes      | 1–4,000 characters. |
| `citations` | yes      | 1–8 citations for `answer`/`summary`; exactly `[]` for `insufficient`. |
| `chunkId`   | yes      | A chunk returned by retrieval for this request. |
| `quote`     | yes      | 2–500 characters copied from the cited chunk. |

An `insufficient` candidate is accepted by validation but `groundAnswer` and
`groundSummary` convert it to a `no-evidence` failure with the standard honest
message. This lets the model refuse when the retrieved context does not answer
the question without inventing a citation.

Citation source metadata (`sourceId`, `sourceName`, `page`) is taken from the
retrieved chunk, never from model output. The validator rewrites `quote` to the
exact source span (with whitespace normalized), so the UI can show the text that
was actually cited.

A citation is rejected when:

- the chunk was not retrieved for this request;
- the chunk id is duplicated;
- the quote is empty, too long, or contains control characters;
- the quote does not occur in the cited chunk, even after case/diacritic
  folding.

The whole answer is discarded if any citation is invalid. The caller receives
`GROUNDING_MESSAGES.unverifiedAnswer`, not a partially trusted answer.

## Evidence-free and unsupported questions

`groundAnswer` and `groundAnswerJson` return a failure instead of an answer when:

- the query has no searchable terms (`no-query`);
- no chunk matches (`no-evidence`);
- the model returns invalid JSON (`invalid-json`);
- the answer envelope is malformed or cites unverifiable text
  (`unverified-answer`).

For `no-evidence`, the user-facing message is:

> I couldn't find enough evidence in the selected PDF to answer that.

The local model is still responsible for recognizing that retrieved context does
not answer the question. This layer guarantees that the model cannot invent a
citation or quote; it does not claim to prove semantic entailment without the
model.

## Summaries

`groundSummary` samples page groups evenly across the document (default 12 pages,
maximum 24) and validates a `kind: "summary"` envelope against those chunks.
`truncated` reports that some pages were omitted. A summary can only cite the
sampled context; this keeps long documents bounded and prevents the model from
citing pages it was never given.

`selectSummaryChunks` can be used directly when the caller needs the sampled
context before generating the summary.

## Opening a cited page

`Citation` contains `sourceId` and `page`, but no filesystem path. The session UI
resolves `sourceId` against its in-memory workspace, then opens the existing
`PageLightbox` at that page. See [assistant-sessions.md](assistant-sessions.md)
for the optional model gate, clear/cancel lifecycle and keyboard behavior.

```ts
import { citationTarget } from "@/lib/assistant/citations";

const target = citationTarget(answer.citations[0]);
// target: { sourceId: "f1", page: 4 }
```

`formatCitation` and `formatCitationList` produce display labels such as
`annual-report.pdf, p. 4` and `annual-report.pdf, pp. 3–4; appendix.pdf, p. 7`.

## Limits

| Limit | Value |
| ----- | ----- |
| Query | 1,000 characters |
| Answer text | 4,000 characters |
| Quote | 500 characters |
| Citations | 8 |
| Retrieved chunks | 6 default / 12 max |
| Retrieved context | 12,000 default / 24,000 max characters |
| Summary pages | 12 default / 24 max |
| Answer JSON | 64 KiB UTF-8 |
| Chunks per request | 5,000 |
| Indexed characters | 2,000,000 |

## API

```ts
import {
  groundAnswer,
  groundAnswerJson,
  groundSummary,
  parseGroundedAnswerJson,
  validateGroundedAnswer,
} from "@/lib/assistant/citations";
import { retrieveChunks } from "@/lib/assistant/retrieval";

const result = groundAnswerJson("What was the revenue?", chunks, modelText);

if (result.ok) {
  result.answer.text;
  result.answer.citations;
} else {
  result.reason;   // "no-query" | "no-evidence" | "invalid-json" | "unverified-answer"
  result.message;  // user-facing, honest failure copy
}
```

`validateGroundedAnswer` accepts an already-parsed value and never throws. It
takes the retrieved chunks, not the whole document, so citations cannot escape
the context used for the answer. `parseGroundedAnswerJson` enforces the 64 KiB
limit, accepts an optional UTF-8 BOM, and rejects markdown fences rather than
stripping them.

## Fixtures and tests

- Fixtures: `src/lib/assistant/fixtures/grounding.json` — multilingual chunks,
  retrieval queries, valid answers and invalid citation cases.
- Tests: `src/lib/assistant/grounding.test.ts` — deterministic retrieval,
  top-k/context bounds, Unicode matching, citation resolution, invalid quotes,
  JSON limits, summary sampling and citation formatting.
- No test or production path performs a network request.

## Versioning

`version` is exact. A breaking change to the answer envelope must increment the
version and keep the old validator available for stored sessions. Additive
optional fields can remain in v1 only if the closed-field validation and this
document are updated together.

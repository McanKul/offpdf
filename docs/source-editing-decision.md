# Existing source-content editing decision

Status: concluded on 2026-10-04 for #11 and #33.

## Decision

OffPDF should not present existing PDF text or images as generally editable.
The read-only classifier is useful as an evidence gate, but a `supported` result
does not authorize a Save action. It is not connected to a Tauri command, React,
or a mutation path.

The only candidates demonstrated by the committed corpus are:

| Fixture | Candidate operation | Boundary |
| --- | --- | --- |
| `text-tj` | Replace the complete `Tj` string | Same simple Helvetica font; no substring edit or reflow |
| `text-tj-kerned` | Replace the complete `TJ` array | Preserve explicit positioning; no paragraph reflow |
| `image-unique` | Replace one Image XObject stream | The object must not be shared, nested, inline, or masked |

These remain research candidates. They need a separate mutation prototype and a
publish gate that proves the source operator or image object was replaced. The
existing overlay validation gate cannot prove that: a white rectangle plus new
text can leave the old searchable content in the PDF.

## Compatibility matrix

| Structure | Classifier result | Reason or note |
| --- | --- | --- |
| Simple `Tj` text | Supported candidate | Whole operand only; demonstrated with Helvetica |
| Kerned `TJ` text | Supported candidate | Whole array only; spacing is included in bounds |
| CID text with ToUnicode | Unsupported | `AMBIGUOUS_UNICODE`; decoding does not prove that replacement text can be encoded |
| CID text without ToUnicode | Unsupported | `NO_TOUNICODE` |
| Subset or custom-encoded simple font | Unsupported | `SUBSET_FONT` or `CUSTOM_ENCODING` |
| Type 3 or vertical text | Unsupported | `TYPE3` or `VERTICAL` |
| Raised text or non-default rendering mode | Unsupported | `TEXT_RISE` or `TEXT_RENDER_MODE` |
| Rotated, skewed, clipped, patterned, or nested text | Unsupported | Explicit geometry/content reason |
| Unique Image XObject | Supported candidate | One independently owned image object |
| Reused, nested, inline, or masked image | Unsupported | `SHARED_XOBJECT`, `NESTED_FORM`, `INLINE_IMAGE`, or `MASKED_IMAGE` |
| Rotated page, offset crop origin, or non-default UserUnit | Unsupported when paint is present | `GEOMETRY` |
| Encrypted, signed, stale, oversized, or malformed source | Rejected | Recoverable file/content error; no silent fallback |

The corpus contains 18 synthetic, license-safe PDFs (14,624 bytes total). It covers
the structures above but is not representative of every Office, browser,
scanner, or CAD producer. Passing it is evidence for the bounded subset, not a
claim that arbitrary PDFs are editable.

## Bounds and failure behavior

The classifier reads and hashes one bounded source snapshot. Raw cross-reference
data, object values, object streams, decoded content, operands, operations,
recursive containers, graphics-state depth, Form recursion, and occurrence count
are capped before their respective parser or traversal can grow without bound.
Unsupported filters and unprovable inline-image boundaries fail closed. Malformed
input returns an application error instead of falling back to compressed bytes
or a visual overlay.

The implementation was built in small reviews: #97 and #117–#123. The fixture
corpus is #32; output validation is #34.

## Performance and packaging evidence

Measurement environment: macOS arm64 (Darwin 25.4.0), Rust 1.96.0, one test
thread, debug test harness, 2026-10-04.

| Measurement | Result |
| --- | --- |
| Classifier and preflight tests | 68 passed |
| Test-reported execution time | 0.13 s |
| Wall time measured with `/usr/bin/time -l` | 0.14 s |
| Peak resident set size | 21,381,120 bytes (about 20.4 MiB) |
| Full Rust library regression suite | 325 passed |
| New native sidecar payload | 0 bytes |
| Fixture bytes included in Tauri resources | 0 bytes |

This is a regression-harness measurement over a small synthetic corpus, not a
production throughput or worst-case memory benchmark. The exact signed-installer
delta was not isolated: lopdf 0.34 was already a direct OffPDF dependency before
the classifier, and the fixture corpus is not bundled. No new native runtime or
sidecar was added. A release-size comparison should be repeated only if this
currently unexposed classifier is wired into the product.

The full Rust suite passes locally on macOS and in the Ubuntu GitHub workflow.
The classifier has not received a dedicated Windows execution run. Because the
current path is Rust code linked into the existing application, it adds no
separate Windows or macOS signing target.

## Engine choice

| Concern | Bounded lopdf prototype | PDFium sidecar |
| --- | --- | --- |
| Current implementation | Complete for the committed classifier corpus | Not implemented |
| Native code boundary | None added | Native library and process boundary required |
| Crash isolation | Runs in-process; bounded safe-Rust preflight reduces parser exposure | Must run out of process before accepting untrusted PDFs |
| Package and signing work | No new native artifact | Per-platform binary packaging and signing; not measured |
| Evidence gained for this spike | Deterministic reasons and locators across the corpus | No additional evidence yet |

Stay on lopdf 0.34 for the read-only classifier. A PDFium prototype is not
justified by the current corpus and was deliberately not built. If a future
fixture cannot be classified safely with lopdf and materially blocks a supported
subset, evaluate PDFium in a separate issue and require an out-of-process
protocol before loading untrusted documents.

## Product boundary

- Keep overlay text and images described as newly added content, never as a
  replacement for existing source content.
- Use secure redaction when the goal is to remove existing content.
- Show a specific unsupported reason; do not flatten, cover, or silently fall
  back.
- Do not expose the classifier as an editing permission until a narrow mutation
  issue proves replacement, output validation, stale-source handling, and
  sibling-destination publishing for its exact subset.

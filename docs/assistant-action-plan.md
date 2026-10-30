# Assistant action-plan schema (v1)

The private local assistant may propose PDF work only as a **versioned JSON action
plan**. The app validates the plan before showing a preview; this module does not
execute anything and does not load a model.

The schema lives in `src/lib/assistant/actionPlan.ts`. Option defaults live in
`src/lib/toolDefaults.ts`, which the manual tool pages also use.

## Security model

A plan is untrusted model output. Validation is closed and fail-closed:

- Unknown tools, fields and enum values are rejected.
- Shell commands, executables, arguments and environment variables are rejected.
- Inputs are **1-based workspace indexes**, never file paths.
- `outputName` is a **file name only**. Path separators, drive letters, `.`/`..`,
  trailing dots/spaces and Windows-reserved names are rejected.
- Passwords are never accepted from the model. `protect` and `unlock` expose
  secret fields that the confirmation UI must collect from the user.
- Plans are capped at 8 actions, 50 inputs, 5,000 pages per list, 100 ranges,
  100 rotations, 64 KiB of JSON, 200 characters for output names and 500
  characters for free text.

## Envelope

```json
{
  "version": 1,
  "actions": [
    {
      "tool": "crop",
      "inputs": [1],
      "options": { "left": 10, "top": 5, "right": 10, "bottom": 5 },
      "outputName": "report-cropped.pdf"
    }
  ]
}
```

Common fields:

| Field        | Required | Meaning                                                        |
| ------------ | -------- | -------------------------------------------------------------- |
| `version`    | yes      | Exactly `1`. Any other number/type is rejected.                |
| `actions`    | yes      | 1–8 actions. Actions are independent; v1 does not chain them.  |
| `tool`       | yes      | One of the supported action ids below.                         |
| `inputs`     | yes      | 1-based indexes into the workspace snapshot. No duplicates.    |
| `options`    | no       | Tool options. Unknown keys are rejected. Missing keys use the shared defaults. |
| `outputName` | single-output tools | File name only. Multi-file tools must omit it. |

Page numbers refer to the combined document formed by `inputs` in the given
order. If page counts are supplied to the validator and all are known, page
references are bounds-checked. If any page count is unknown, bounds checks are
skipped (the execution layer must still verify them).

## Supported actions

All actions map to a tool already exposed by the app. Defaults are the same
constants used by the manual pages.

| Tool | Inputs | Options and defaults | Output |
| ---- | ------ | -------------------- | ------ |
| `merge` | 2–50 | — | single PDF, default `merged.pdf` |
| `split` | 1–50 | `mode` = `everyN` (default), `everyN` = 10; or `mode` = `ranges` with ascending, non-overlapping `{start,end}` ranges | multiple PDFs |
| `reorder` | 1–50 | `order` (exact output order) **or** `remove` (pages to drop); `rotate` = `[{pages, angle: 90\|180\|270}]`. Pages are unique; duplication is not supported in v1. A page can only have one rotation. Use only one of `order`/`remove`; at least one change is required. | single PDF, default `edited.pdf` |
| `pageNumbers` | 1–50 | `position` = `bottom-center`, `start` = 1, `format` = `plain`, `prefix` = `""`, `padWidth` = 6. `prefix`/`padWidth` only apply to `bates` and `bates-date`. | single PDF, default `numbered.pdf` |
| `watermark` | 1–50 | `text` (required, 1–200 chars), `opacity` = 0.3 (0.05–1) | single PDF, default `watermarked.pdf` |
| `crop` | 1–50 | `left`, `top`, `right`, `bottom` = 5 (0–<50 % each); at least one edge > 0 | single PDF, default `cropped.pdf` |
| `stamp` | 1–50 | `text` (required, 1–200 chars), `page` = 1, `anchor` = `bottom-right`, `color` = `red`, `size` = 0.04 (`0.025`/`0.04`/`0.06`) | single PDF, default `stamped.pdf` |
| `compress` | 1–50 | `mode` = `target` with `targetMb` = 20, or `mode` = `keepText` | single PDF, default `compressed.pdf` / `optimized.pdf` |
| `images` | 1–50 | `format` = `png` (`png`/`jpg`), `dpi` = 150 (`96`/`150`/`300`) | multiple images |
| `repair` | 1–50 | — | single PDF, default `repaired.pdf` |
| `protect` | 1–50 | —; password is a **user secret** | single PDF, default `protected.pdf` |
| `unlock` | exactly 1 | —; password is a **user secret** | single PDF, default `unlocked.pdf` |
| `ocr` | 1–50 | `languages` (required): 1–8 Tesseract codes, e.g. `["eng","tur","chi_sim"]` | single PDF, default `searchable.pdf` |
| `pdfa` | 1–50 | — | single PDF, default `archive.pdf` |
| `pdfToOffice` | 1–50 | `format` = `docx` (`docx`/`pptx`) | single file, default `converted.docx` / `converted.pptx` |
| `textExport` | exactly 1 | `scope` = `all`, or `scope` = `range` with `from` (default 1) and required `to` | single TXT, default `document.txt` |
| `nup` | 1–50 | `mode` = `2up` (`2up`/`4up`/`booklet`), `paper` = `a4`, `orientation` = `auto`. A booklet needs at least 3 pages when the count is known. | single PDF, default `layout.pdf` |
| `poster` | 1–50 | `page` = 1, `paper` = `a4`, `orientation` = `auto`, `overlapMm` = 10 (0–50), `marks` = `true` | single PDF, default `poster.pdf` |
| `metadata` | exactly 1 | `clearAll` = `false`, or `fields` with any of `title`, `author`, `subject`, `keywords`, `creator`, `producer`. Do not combine `clearAll` with `fields`; a plan must change something. | single PDF, default `metadata.pdf` |

### Deliberately excluded

These tools exist in the app but cannot be planned safely in v1:

| Tool | Reason |
| ---- | ------ |
| `editPdf` | Interactive text/shape/form editing, not a deterministic one-shot action. |
| `compare` | Read-only; produces no output file. |
| `blankPages` | Needs visual review before pages are removed. |
| `delete`, `extract`, `rotate` | Legacy action ids; use `reorder` with `remove`, `order` or `rotate`. |
| `optimize` | Legacy action id; use `compress` with `mode: "keepText"`. |
| `officeToPdf` | Workspace entries are PDFs, not the original Office files required for conversion; use the Office to PDF tool directly. |

## Validation API

```ts
import {
  parseActionPlanJson,
  validateActionPlan,
  type PlanValidationContext,
} from "@/lib/assistant/actionPlan";

const context: PlanValidationContext = {
  fileCount: 2,
  pageCounts: [12, 4],
  installedOcrLanguages: ["eng", "tur"],
};

const result = parseActionPlanJson(modelText, context);

if (result.ok) {
  // result.plan.actions is normalized: defaults are applied and output names
  // are safe file names.
} else {
  // result.issues: { path, code, message }[]
}
```

`validateActionPlan` accepts an already-parsed value and never throws.
`parseActionPlanJson` enforces the 64 KiB limit, accepts an optional UTF-8 BOM,
and rejects markdown fences rather than stripping them.

## Fixtures and tests

- Valid plans: `src/lib/assistant/fixtures/valid.json` (one or more per supported tool).
- Invalid plans: `src/lib/assistant/fixtures/invalid.json` (with the expected
  issue code and optional validation context).
- Tests: `src/lib/assistant/actionPlan.test.ts` cover unknown fields/tools/enums,
  secrets, path traversal, defaults, page bounds, duplicate outputs, limits and
  all fixtures.
- Adversarial tests: `src/lib/assistant/actionPlan.security.test.ts` cover shell
  commands and hidden fields, paths and input indexes outside the selected
  workspace, invalid/overlapping/reversed/excessive page ranges, ambiguous
  options and false success claims. They also assert that parsing untrusted model
  output never invokes a Tauri command.

## Versioning

`version` is exact. A future breaking change (new required fields, changed
semantics, chaining, or action renaming) must increment the version and keep the
old validator available for stored plans. Additive optional fields can remain in
v1 only if the closed-field validation is updated and documented here.

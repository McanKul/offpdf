# Edit model contract

This document describes the reusable visual PDF editor canvas model used by
OffPDF. Overlay export (Edit PDF) consumes this contract; other tools must not
invent a second coordinate system.

## What this module is

- A **typed, JSON-serializable** description of draft objects on PDF pages.
- Pure **viewport ↔ PDF** coordinate transforms.
- An **undo/redo** reducer for editor sessions.
- Kinds: `text`, `image` (filesystem path), `line`, `ink`, and closed vector
  shapes such as rectangles, ellipses, arrows, and polygons.
- `sourceText`: a change to a line of existing text (Edit text). It is not an
  overlay; see [Source text edits](#source-text-edits-kind-sourcetext).

## What this module is not

- Apart from `sourceText`, objects never touch existing page content
  operators. `sourceText` changes them only through the Rust text-edit engine,
  never in the frontend.
- **Links** (`kind: "link"`) are PDF `/Annots`, not overlay stamps. They use
  the same unrotated `EditObject.rect` space. Overlay paint skips them; Save
  rewrites dest `/Link` dictionaries after `qpdf --overlay`.
- **Redaction** (`kind: "redact"`) is not an overlay stamp. On Save, pages with
  ≥1 redact object are rasterized in place (fill and optional typed label
  burned into the image). Overlay paint skips them. Unredacted pages keep their
  source streams. Leftover `/Annots`, form `/V`, and attachments warn only.
- It does **not** hold source PDF bytes. Paths and per-page bytes stay in the
  render layer (same pattern as `pagePdf`).
- Image **bytes** are not stored in the document — only a local path (plus a
  session-only preview URL stripped before IPC).

## Coordinate spaces

| Space | Origin | Units | Stored? |
| --- | --- | --- | --- |
| PDF page space | Absolute unrotated user space. Preview and export mapping subtract the **visible** box (CropBox ∩ MediaBox). `/UserUnit` is copied onto overlay pages. | PDF points | **Yes** — `EditObject.rect` |
| Display page space | Lower-left of the page after applying `/Rotate` | points | Internal only |
| CSS / viewport | Top-left of the rendered page element | CSS pixels | Transient UI |

`/Rotate` (0 / 90 / 180 / 270) affects **display only**. Moving an object and
then viewing the same page at another rotation still exports the same PDF
coordinates.

### API

```ts
pdfToViewport(point, mapping) → cssPoint
viewportToPdf(cssPoint, mapping) → point
pdfRectToViewport(rect, mapping) → cssRect
viewportRectToPdf(cssRect, mapping) → rect
displayedSize(geometry) → { w, h }
```

`ViewportMapping` is `{ cssWidth, cssHeight, geometry }` where `geometry`
includes `box` (pdf.js visible view), optional `userUnit`, `rotate`, and `pageIndex`.
Rust also reads qpdf's native alignment box (raw page TrimBox, not inherited or
clipped to Media → Crop → Media). When that box or MediaBox differs from the
visible box, export normalizes a temporary working page before composition; the
preview never observes TrimBox.

## EditDocument

```ts
{
  version: 1,
  objects: EditObject[],
  selectedIds: string[]
}
```

Each object has at least:

- `id` — stable string (UI uses `crypto.randomUUID()`)
- `kind` — `"text" | "image" | "rect" | "line" | "ink"`
- `pageIndex` — 0-based index within the editor session (assembled workspace
  order). Identity across add/remove/reorder is `${file.uid}#${page}` from
  `useCombinedDoc`; remap `pageIndex` by those keys. Do not store `pageKey` in
  the document.
- `rect` — `{ x, y, w, h }` in unrotated PDF points
- `keepAspect` — optional; Square/Circle tools set this so resize and W×H stay 1:1

Source path and 1-based page number are **session props**, not part of the
document, so the same model can be reapplied after reordering tools assemble a
job.

Existing AcroForm fill is **not** an overlay stamp. `list_form_fields` walks
catalog `/AcroForm` on the **source path** (never `pagePdf --empty`). Widget
`/Rect [llx lly urx ury]` is listed as `{x: llx, y: lly, w, h}` in the same
unrotated user space as `EditObject.rect`. Preview chrome maps those rects with
`pdfRectToViewport` / `geometry.box`. Live values stay in a session map (field
name → value), not on the undo stamp stack.

## History rules

- `ADD` / `UPDATE` / `DELETE` create undo steps.
- `SELECT` / `CLEAR_SELECTION` do **not**.
- Move/resize uses `BEGIN_GESTURE` → many `UPDATE`s → `END_GESTURE` so one drag
  undoes as a single step.
- History depth is capped (`MAX_HISTORY = 100`).
- Adding a PDF (keys only grow) remaps indices if needed and **keeps undo**.
- Removing a PDF with no edits on its pages remaps survivors and keeps undo.
- Removing a PDF that still has objects **or undo history** on its pages prompts
  first; cancel leaves workspace and edits unchanged. Confirm remaps present and
  **clears undo** (dropped indices). The editor session lives on the page so
  removing an earlier file cannot unmount and wipe later pages. Do not wipe the
  session with a concatenated `resetKey`.

Markup kinds `note` / `highlight` / `underline` / `strikeout` / `markupInk` are
session `/Annots` dictionaries (not overlay stamps). They use the same
unrotated `rect` space as stamps. Overlay paint skips them; Save copies every
existing annot through and appends or removes only session `/NM` dicts. Draw
`kind: "ink"` stays a content-stream stroke. Flatten is opt-in
`qpdf --flatten-annotations=all` (default off).

## Source text edits (`kind: "sourceText"`)

A `sourceText` object records a change to one line of text that is already in a
source PDF. It is **not** an overlay: Save rewrites that line's show operators
inside the page's own content stream. The user guide, the reason codes and the
full verification design are in [`docs/EDIT_TEXT.md`](../../../docs/EDIT_TEXT.md).

```ts
{
  id, kind: "sourceText", pageIndex, rect,   // rect = the line's box after the change (verdict newRect)
  locked: true,                              // never moved, resized, rotated, copied or pasted
  runId,                                     // the engine's line id on that source page
  sourceFingerprint,                         // content hash of the source file when the line was read
  sourcePageIndex,                           // 0-based page in the source file
  originalText, text,
  style: { sizePt?, face?, fill?, letterSpacingPt? }   // only fields that differ from the original
}
```

Rules:

- **One object per `(pageIndex, runId)`.** Committing a change upserts it;
  committing the original text with no style change deletes it (a no-op writes
  nothing). Commit, restore and inspector style changes are one undo step each.
- **Locked.** `applyUpdate` accepts only `text` and `style`; it strips `rect`,
  rotation, opacity, `locked`, `runId`, fingerprint and page fields. The objects
  draw nothing in the SVG layer, are excluded from marquee, copy, paste,
  duplicate and nudge, and are never added by `ADD_MANY`.
- **Fingerprint and source page.** Every object carries the source fingerprint
  and `sourcePageIndex`. Reordering pages remaps `pageIndex` and keeps both;
  removing a page drops its changes. If the file changes on disk, the fingerprint
  no longer matches: the editor shows the stale banner, and "Remove these text
  changes" deletes all of that file's changes in one step.
- **Preview bytes.** The canvas shows the page that Save would write: on commit,
  `preview_text_edits` plans the page's changes, writes a one-page copy through
  qpdf and runs the same Phase A checks as Save. The returned one-page PDF is
  rendered in place of the original page (double-buffered, cached per edit
  signature). **Show original** (O) swaps back; it is offered only while a
  preview is on screen (`canShowOriginal`: the page has changes and preview
  bytes). The bytes never leave the page surface and are never stored in the
  document.
- **Save.** `toExportDocument` sends the objects to `edit_pdf_overlays`
  unchanged; Rust ignores `id` and `locked`. Save re-reads each edited source
  (never the editor's cache), checks the fingerprint, plans every change again
  and writes a per-source edited copy **before** assembly. Phase A proves each
  copy; the existing pipeline (assembly, stamps, links, forms, markup, redaction
  on other pages) runs on the edited copies; Phase B proves the final file; then
  the existing #34 validation runs and the file is renamed into place. Clicking
  Save while a line edit is open first finishes it (`finishOpenTextEdit` in
  `textSaveGuards.ts`): Save waits for the commit and runs again on the
  committed objects; a draft that can't be applied stays open, the editor shows
  "Finish or cancel the text change on this page first." and nothing is saved.
  For links, a save whose objects are all `sourceText` counts as empty, so links
  deleted in the editor stay deleted (the L7 rule in `edit_overlay.rs`).
- **Redaction conflict.** A page cannot have both a redaction and a text change
  (redaction turns the page into an image). The editor says so in the redaction
  note; Save refuses with `TEXT_EDIT_ON_REDACTED_PAGE`.
- **A page listed twice** (same file and source page) cannot take text changes;
  the editor shows a banner and Save refuses with `TEXT_EDIT_DUPLICATE_PAGE`.
- **Refused lines** come from inspect with a reason and never become objects.
  Text drawn through a Form XObject arrives as refused runs (`NESTED_FORM`, no
  metrics or style, ids that never match a page run) after the page's own runs.
- **Add text here** (reason dialog) adds an ordinary `text` object whose box is
  `textStampGeometry(run, geometry)`, worked out as displayed: a level line's
  own box grown to one line and four characters; for a line turned, tilted or
  vertical on screen, an upright 10 em × 1.3 em box at the line's start. The box
  is kept on the page.

### Engine architecture and invariants

The Rust side lives in `src-tauri/src/pdf_engine/text_edit/` (module map in
`docs/EDIT_TEXT.md` §16). The invariants every change must keep:

1. **Real source edits only.** A change is a byte splice of show operators in the
   page's own content stream, written by `qpdf --update-from-json`. Never a
   painted-over fake, a flattened or rasterised page, a lopdf re-save, or a write
   to the input.
2. **One bounded snapshot per operation.** The file is read once with a cap,
   hashed, and parsed from the same bytes; qpdf only reads copies of those bytes.
3. **Fail closed.** Every refusal has a code from `text-reasons.json`; nothing is
   skipped or returned partially as success.
4. **Bounded.** Every decompression is capped while inflating, every parser has
   an operation and nesting budget, and each page model has a byte budget
   (`PAGE_MODEL_BYTES_MAX`, 160 MiB) that covers the font models it keeps and its
   #33 Classify pass; a model keeps no lexed operators. Exceeding a page budget
   refuses that page (`PAGE_TOO_COMPLEX`), never the app. The cache sizes models
   by `walk.model_bytes`, and Save rebuilds every page's model in Phase A instead
   of keeping them once they pass `max(2 × file, 16 MiB)`.
5. **Verified before publish.** Plan self-check → qpdf write → Phase A (A0 qpdf
   check, A1 page-map agreement, A2 whole-graph equality, A3 exact edited bytes,
   A4 re-walk at 0.01 pt, A5 Poppler words and pixels) → existing passes →
   Phase B (B0 read, B1 expected content present, B2 original absent, B3 re-walk)
   → #34 → atomic rename. Content-part boundaries must mean the same to every
   reader: one rule (`content/joins.rs`) makes the walker refuse a page whose
   parts meet inside a token, comment or `BX`…`EX` section (`MALFORMED_CONTENT`),
   and #34's joined-form digest and Phase B's wrapper form need the same rule.
6. **No panics in production code**, no forbidden lopdf APIs (GUARD-01), test
   seams only under `#[cfg(test)]`.
7. **One copy source.** Every user-facing sentence is in `sourceTextCopy.ts`
   (Rust builds the same `AppError` text in `reasons.rs`); every Save failure
   ends with "The original file was not changed."

### Edit text contributor workflow

#### Adding a reason code

Codes are append-only. In one change:

1. `text_edit/reasons.rs`: add the variant, its `as_str` spelling, its priority
   slot and its title and body.
2. `src/lib/editor/text-reasons.json`: append the code to its list
   (`reasons_json_matches_enums` cross-locks the order with Rust).
3. `src/lib/types.ts` (union) and `src/lib/editor/sourceTextCopy.ts`
   (`REASON_COPY`, `PAGE_REASON_COPY` or the error rows): `sourceTextCopy.test.ts`
   fails until every code has copy and passes the wording scan.
4. `docs/EDIT_TEXT.md`: add the row inside the ui-copy block of section 6 or 7.
   `docsReasons.test.ts` fails until the doc names the code and quotes the copy
   exactly.
5. CLS-H22 derives the classifier's frozen list from the JSON; update its legacy
   assertion only if a legacy #33 name changes (it must not).

#### Engines and the skip policy

Tests that need qpdf ≥ 11, `pdftotext` and `pdftoppm` call
`testkit::engines_or_skip`, which prints `skip: … not available` and returns
when a tool is missing. With `OFFPDF_REQUIRE_ENGINES=1` a missing tool fails the
test instead. CI installs qpdf and Poppler, sets the variable and fails if any
test prints a `skip:` line. Locally (PR #98 convention):

```bash
cd src-tauri
cargo test --lib -j 6 -- --nocapture 2>&1 | grep -c 'skip:'   # must print 0
OFFPDF_REQUIRE_ENGINES=1 cargo test --lib -j 6 text_edit
```

#### Benchmarks and generated fixtures

```bash
cd src-tauri
cargo test --release --lib -j 6 text_edit::bench -- --ignored --nocapture --test-threads=1   # BENCH-01/02
OFFPDF_UPDATE_GOLDEN=1 cargo test --lib meas_01_estimate_golden  # rewrites text-measure-golden.json (MEAS-01)
OFFPDF_UPDATE_GOLDEN=1 cargo test --lib dto_01_serialised        # rewrites text-edit-dto-contract.json
```

The two JSON files under `src/lib/editor/__fixtures__/` are read by
`sourceTextGolden.test.ts` and `sourceTextContract.test.ts`, which keep the
frontend's width estimate and DTO types in step with Rust (the contract test
derives each DTO's schema from `types.ts`, so a drift also fails `npm run
typecheck`). BENCH-SIZE is measured as described in `docs/EDIT_TEXT.md` §12.

#### Adding a font or producer fixture

Fixtures are generated in memory by the Rust test kit; no PDFs are committed
for Edit text (`fixtures/source-edit/` does not grow).

- **Font programs:** `text_edit/testkit/{ttf,cff,type1}.rs` build TrueType, CFF
  and Type1 programs with chosen glyphs (`TtfBuilder::glyph(name, outline,
  advance)`, …). `testkit/fonts.rs` wraps them in PDF font dictionaries
  (`SimpleFont`, `Type0Font`, `add_simple`, `add_type0`, ToUnicode helpers) and
  loads them (`load_simple`, `load_type0`). Real programs already in the repo may
  be used read-only: `src-tauri/resources/fonts/NotoSans-Regular.ttf`,
  `public/pdfjs/standard_fonts/LiberationSans-*.ttf`, and the
  `public/pdfjs/standard_fonts/Foxit*.pfb` files (these are bare CFF programs,
  not Type1).
- **A new font class:** add the class to `fonts/` with a glyph-presence proof
  from the program, fixtures and FONT tests in `fonts/tests*.rs`, then add a case
  to IND-01 (`tests_independent.rs`) so an honest edit in that font passes Phase A
  including the Poppler checks, and a line to E2E-05 (`tests_e2e/fonts.rs`) so it
  saves through the real export.
- **A producer shape:** add a builder to `testkit/producers/` (office-style files
  in `office.rs`, geometry and syntax edges in `edges.rs`, file-level cases in
  `files.rs`, tagging in `tagging.rs`), add its smoke test in
  `tests_walk/producers.rs`, assert its editable and refused lines in a RUN or
  CLS-H test, and run it through E2E so the Save path covers it. Add the row to
  the compatibility matrix in `docs/EDIT_TEXT.md` §11 with the test ids.
- Run `OFFPDF_REQUIRE_ENGINES=1 cargo test --lib -j 6 text_edit` and check that
  `cargo test --lib -j 6 -- --nocapture 2>&1 | grep -c 'skip:'` prints 0.

## How export consumes this

1. Read `EditDocument.objects` for the chosen pages.
2. Map each `rect` / point from unrotated PDF space through the **visible box**
   and `/Rotate` into displayed overlay space. Overlay pages copy destination
   `/UserUnit` and use the transformed absolute visible box so qpdf maps 1:1.
   Stored coordinates stay absolute.
3. Build a hand-rolled overlay PDF (embedded Noto Sans, vector ops, image
   XObjects) and `qpdf --overlay` it onto the **primary source**, not an empty
   rebuild. A single full-range file is `original --overlay overlay -- dest`
   (bookmarks, Info/XMP, AcroForm stay). A subset or multi-file job uses the
   first file as infile with `--pages . <spec> …` — same compromise as Optimize.
   Never `qpdf --empty --pages`. If qpdf's native alignment would differ from
   the preview, normalize Media/Crop/Trim together on temporary source copies,
   compose, then restore the original page boxes on the output. Never change
   the user's source file.
4. Keep offline path-based processing; never put full PDF bytes into React state
   for export.
5. Preserve “original file is never overwritten.”

## Accessibility

The canvas exposes the active page's object list so selection and deletion work
without pointer-only interaction. Selection-driven actions never affect hidden
pages. Keyboard: Delete/Backspace, Escape, undo/redo chords, arrow nudge. Hand
tool (H) and hold-Space pan the zoomed page; trackpad scroll on the stage still
works. The editor opens on the **Select** tool.

### Edit text keyboard contract

Single-key shortcuts work when focus is not in a text field, the format bar or
the reason dialog.

| Where | Key | Action |
| --- | --- | --- |
| Editor | **E** | Edit text tool (`aria-keyshortcuts="E"`; **H** is the Hand tool) |
| Editor, page with text changes and a preview on screen | **O** | Show original (toggle, `aria-pressed`) |
| Text layer (one Tab stop, roving focus over line buttons) | ↓ / ↑ | next / previous line in reading order |
| | → / ← | next / previous text block in reading order (moves within a visual line, then on to the next or previous one) |
| | Home / End | first / last text block on the page |
| | Page Up / Page Down | 10 lines back / forward |
| | Enter, F2 or Space | edit the line with all text selected, or open the reason dialog for a refused line |
| | Delete / Backspace | restore the original of a changed line (one undo step) |
| | Esc | leave the layer |
| | ⌘/Ctrl+Z, ⌘/Ctrl+Shift+Z, ⌘/Ctrl+Y | undo / redo (existing handler) |
| Line editor | Enter | Done (commit after the check) |
| | Esc | cancel the draft |
| | Tab | into the format bar (first stop: Smaller) |
| | ⌘/Ctrl+B, ⌘/Ctrl+I | bold / italic, or write why it is unavailable |
| | ⌘/Ctrl+Shift+. and ⌘/Ctrl+Shift+, | size +0.5 / −0.5 pt |
| Format bar (`role="toolbar"`) | ← / → | move between controls |
| | Enter / Space | activate |
| | Esc, Shift+Tab | back to the line editor |
| Reason dialog | Esc | close; focus returns to the line |

Space-to-pan is suppressed while focus is in the text layer, the line editor, the
format bar or the reason dialog, so Space activates the focused control. Enter
and Esc are ignored while an input method is composing. Lines are real
`<button>`s labelled "Edit text: …", "Edited text: {new}. Original: {old}" (one
full stop when the new text already ends a sentence: `editedRunLabel`) or, for
refused lines, the text with `aria-disabled` and a description naming the
reason. Commit and restore are announced; focus returns to the line after a
commit, a restore, an undo or closing the dialog.

## Offline / privacy

No network, telemetry, or cloud persistence. Temporary page bytes used for
preview follow existing large-file safeguards (single page via `pagePdf`).

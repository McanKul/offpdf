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

## What this module is not

- It does **not** edit existing page content operators.
- **Links** (`kind: "link"`) are PDF `/Annots`, not overlay stamps. They use
  the same unrotated `EditObject.rect` space. Overlay paint skips them; Save
  rewrites dest `/Link` dictionaries after `qpdf --overlay`.
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
works.

## Offline / privacy

No network, telemetry, or cloud persistence. Temporary page bytes used for
preview follow existing large-file safeguards (single page via `pagePdf`).

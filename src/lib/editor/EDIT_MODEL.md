# Edit model contract (issue #6)

This document describes the reusable visual PDF editor canvas model used by
OffPDF. Later tools (overlays, annotations, form fill, redaction) consume this
contract; they must not invent a second coordinate system.

## What this module is

- A **typed, JSON-serializable** description of draft objects on PDF pages.
- Pure **viewport ↔ PDF** coordinate transforms.
- An **undo/redo** reducer for editor sessions.

## What this module is not

- It does **not** write objects into a PDF. Export/save is issue #7+.
- It does **not** edit existing page content operators.
- It does **not** hold source PDF bytes. Paths and per-page bytes stay in the
  render layer (same pattern as `pagePdf` + `RefLightbox`).

## Coordinate spaces

| Space | Origin | Units | Stored? |
| --- | --- | --- | --- |
| PDF page space | Lower-left of the **visible page box** (CropBox ∩ MediaBox, else MediaBox), unrotated | PDF points | **Yes** — `EditObject.rect` |
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
includes `box`, `rotate`, and `pageIndex`.

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
- `kind` — `"rect"` today; later `"text" | "image" | …`
- `pageIndex` — 0-based index within the editor session
- `rect` — `{ x, y, w, h }` in unrotated PDF points

Source path and 1-based page number are **session props**, not part of the
document, so the same model can be reapplied after reordering tools assemble a
job.

## History rules

- `ADD` / `UPDATE` / `DELETE` create undo steps.
- `SELECT` / `CLEAR_SELECTION` do **not**.
- Move/resize uses `BEGIN_GESTURE` → many `UPDATE`s → `END_GESTURE` so one drag
  undoes as a single step.
- History depth is capped (`MAX_HISTORY = 100`).

## How #7 should consume this

1. Read `EditDocument.objects` for the chosen pages.
2. For each object, use `rect` + page geometry (box / rotate) to build overlay
   content in PDF user space.
3. Keep offline path-based processing; never put full PDF bytes into React state
   for export.
4. Preserve “original file is never overwritten.”

## Accessibility

The canvas exposes an object list (or equivalent) so selection and deletion work
without pointer-only interaction. Keyboard: Delete/Backspace, Escape, undo/redo
chords, arrow nudge.

## Offline / privacy

No network, telemetry, or cloud persistence. Temporary page bytes used for
preview follow existing large-file safeguards (single page via `pagePdf`).

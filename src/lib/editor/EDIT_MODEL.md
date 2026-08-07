# Edit model contract

This document describes the reusable visual PDF editor canvas model used by
OffPDF. Overlay export (Edit PDF) consumes this contract; other tools must not
invent a second coordinate system.

## What this module is

- A **typed, JSON-serializable** description of draft objects on PDF pages.
- Pure **viewport ↔ PDF** coordinate transforms.
- An **undo/redo** reducer for editor sessions.
- Kinds: `text`, `image` (filesystem path), `rect`, `line`, `ink`.

## What this module is not

- It does **not** edit existing page content operators.
- It does **not** hold source PDF bytes. Paths and per-page bytes stay in the
  render layer (same pattern as `pagePdf`).
- Image **bytes** are not stored in the document — only a local path (plus a
  session-only preview URL stripped before IPC).

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
- `kind` — `"text" | "image" | "rect" | "line" | "ink"`
- `pageIndex` — 0-based index within the editor session
- `rect` — `{ x, y, w, h }` in unrotated PDF points
- `keepAspect` — optional; Square/Circle tools set this so resize and W×H stay 1:1

Source path and 1-based page number are **session props**, not part of the
document, so the same model can be reapplied after reordering tools assemble a
job.

## History rules

- `ADD` / `UPDATE` / `DELETE` create undo steps.
- `SELECT` / `CLEAR_SELECTION` do **not**.
- Move/resize uses `BEGIN_GESTURE` → many `UPDATE`s → `END_GESTURE` so one drag
  undoes as a single step.
- History depth is capped (`MAX_HISTORY = 100`).

## How export consumes this

1. Read `EditDocument.objects` for the chosen pages.
2. Map each `rect` / point from unrotated PDF space through the visible box
   (`CropBox ∩ MediaBox`) and `/Rotate` into **displayed overlay space**
   (same size as stamp/watermark overlays).
3. Build a hand-rolled overlay PDF (embedded Noto Sans, vector ops, image
   XObjects) and `qpdf --overlay` it onto the assembled document.
4. Keep offline path-based processing; never put full PDF bytes into React state
   for export.
5. Preserve “original file is never overwritten.”

## Accessibility

The canvas exposes an object list (or equivalent) so selection and deletion work
without pointer-only interaction. Keyboard: Delete/Backspace, Escape, undo/redo
chords, arrow nudge. Hand tool (H) and hold-Space pan the zoomed page; trackpad
scroll on the stage still works.

## Offline / privacy

No network, telemetry, or cloud persistence. Temporary page bytes used for
preview follow existing large-file safeguards (single page via `pagePdf`).

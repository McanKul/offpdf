# Changelog

All notable project changes should be documented here.

## Unreleased

### Fixed

- HEIC/HEIF import refuses to copy pixels when the decoded plane size does not match the image handle, so a tiled or rotated phone photo cannot walk off the buffer. Invalid files still return an in-app error.

## 0.3.0 - 2026-08-15

### Added

- Edit PDF: add text (including Turkish), PNG/JPEG images, rectangles, lines and freehand drawings on a live page preview. Saves a new overlay PDF via qpdf; the original file is never overwritten. Noto Sans is bundled (SIL OFL) for Unicode overlay text.
- Edit PDF: Hand tool (H) click-drags a zoomed page to pan; hold Space for a temporary hand. Trackpad scroll still works.
- Edit PDF: Square and Circle in the shape picker stay 1:1; Rectangle and Ellipse stay free (hold Shift for a perfect ratio). Width and height can be typed; the lock keeps the ratio when resizing.

### Fixed

- Edit PDF: saving keeps the original as qpdf's primary input, so bookmarks, document metadata, and forms survive a single-file export. Multi-file edits keep catalog data from the first file (same as Optimize).
- Edit PDF: the never-overwrite guard compares file identity (not just path), and save writes to a sibling temp file then renames it into place so a hard-linked output path cannot truncate the original.
- Edit PDF: preview and export share one visible-page geometry model. Offset CropBox/TrimBox pages, `/UserUnit`, and 0°/90°/180°/270° rotation preserve both original content and overlay placement.
- Edit PDF: preview and save share the same image limits (20 MB, 4096 px on a side). Oversized files are rejected from headers before decode; the same image used twice is embedded once; a document-wide byte/pixel budget applies; save can cancel between images.
- Edit PDF: adding or removing a workspace PDF remaps edits by page identity instead of clearing the canvas. Removing a PDF that still has edits asks first.
- Page preview uses a grab cursor for drag-to-pan instead of a zoom-in magnifier. Zoom stays on the toolbar, keyboard shortcuts, and Ctrl/Cmd + wheel.
- Edit PDF keeps selection, copy, delete, duplicate, drag, and arrow-key movement on the active page; hidden-page objects are not changed.
- Edit PDF keeps both edges reachable at high zoom and places output settings, save progress, cancellation, and completed-file actions before the canvas.
- The full-page reader now exposes accessible names for icon-only controls, search, and page thumbnails.
- Output file names enforce the 200-character limit after adding the `.pdf` extension.
- Metadata and bookmark titles decode PDFDocEncoding punctuation and symbols correctly.
- HEIC/HEIF imports validate compressed size and image dimensions before native decoding, avoid an extra full-size RGB copy, and run image conversions serially so large batches cannot multiply peak memory use.

### Contributors

- Thank you to @nonamexishere, @YuukiRitoTeng, @strongdan, and @joyheroes for their contributions to this release.

### Distribution

- The Windows x64 installer is available through GitHub Releases with an accompanying SHA-256 file. It remains clearly marked as unsigned while Authenticode signing is completed and is not linked from offpdf.com.
- The Windows release workflow builds, installs, launches, checks bundled runtimes, and scans the installer with Microsoft Defender before attaching it to a draft release.

## 0.2.2

### Fixed

- Windows: bundle the Microsoft Visual C++ runtime beside the application so OffPDF starts on clean Windows installations after HEIC/HEIF support was added.
- Windows release builds now install and launch the packaged application as a smoke test before publishing it.

## 0.2.1

### Added

- Convert HEIC and HEIF photos, including iPhone and iPad images, to PDF entirely offline with a bundled decoder.

## 0.2.0

### Fixed

- Compress: output pages keep their real physical size in target-size mode, and thin technical-drawing lines survive compression (150 dpi floor, higher minimum JPEG quality).
- Watermark, stamp and page numbers render Turkish and other non-ASCII text correctly.
- Page numbers are positioned correctly on every page size (A4, large-format plans, rotated pages).
- Crop no longer enlarges the visible area on pre-cropped files and trims the correct visual edges on rotated pages.
- Stamp lands at the right position and size on rotated pages.
- Optimize preserves bookmarks and document metadata, and keeps the original when rewriting would grow the file.
- Office conversions: profile paths with spaces no longer crash LibreOffice, outputs are named after the source file instead of merged.docx, concurrent conversions no longer collide, and LibreOffice errors surface in technical details.
- PDF to Excel removed — LibreOffice cannot import PDFs into Calc; the option always failed silently.
- Poster prints rotated pages in the requested sheet orientation.

### Added

- N-up / Booklet: 2-up, 4-up and saddle-stitch imposition.
- Bates numbering: prefix, zero-padded counter and optional date on page numbers.
- Remove blank pages with adjustable sensitivity and thumbnail review.
- Metadata editor with a clear-all sanitize mode.
- PDF to text export (whole document or page range).

### Docs

- Cleaned public documentation for the OffPDF open-source repository.
- Added contributor, security, roadmap, and branding guidance.
- Aligned package metadata with the OffPDF name.

## 0.1.0

- Initial pre-release desktop app.

# Changelog

All notable project changes should be documented here.

## Unreleased

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

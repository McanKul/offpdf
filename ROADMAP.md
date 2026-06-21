# Roadmap

OffPDF is a free, open-source, offline-first desktop app. The roadmap prioritizes
trust, reliable packaging, and performance on large PDFs.

## Current Focus

- Public repository cleanup and contributor-ready documentation.
- Signed and notarized macOS builds.
- Windows installer that works for non-technical users and includes the core
  runtime pieces offline.
- Windows test artifacts from GitHub Actions for early installation testing.
- GitHub Releases with checksums as the canonical binary distribution channel.
- A simple `offpdf.com` landing page with one clear download button per
  platform.

## Current Capabilities

- Merge, split, organize, rotate, delete, crop, stamp, watermark, number, and
  compare PDFs.
- Create poster/tile print layouts for large drawings.
- Convert Office/HTML files to PDF and export PDFs to Office formats.
- Export PDF pages to images.
- Make scanned PDFs searchable with OCR.
- Convert to PDF/A for archiving.
- Compress PDFs with either text-preserving cleanup or target-size rasterization.
- Protect, unlock, and repair PDFs locally.

## Known Limitations

- Release builds are not signed or notarized yet.
- Core tools need `qpdf`; render-heavy tools need poppler; OCR needs Tesseract;
  Office/PDF-A tools need LibreOffice.
- Engine binaries are not committed to the repository. Release packaging still
  needs a clean per-platform bundling flow.
- Lossy compression rasterizes pages, so selectable text is not preserved in
  that mode. Use the text-preserving option when text selection matters.
- Complex Office/PDF conversions are best effort because they depend on
  LibreOffice's import/export behavior.
- Auto-update is intentionally not enabled.

## Near Term

- Finalize app metadata, icons, and public branding under OffPDF.
- Bundle `qpdf` and poppler per platform and document how release artifacts are assembled.
- Add GitHub Actions for build checks and release artifact generation.
- Add checksums to every release.
- Prepare the first public alpha release.

## Later

- Bundle or document optional engine packs for Tesseract and LibreOffice.
- Improve OCR language selection and install guidance.
- Add more robust PDF/A validation after export.
- Add optional, clearly disclosed update checks behind an offline-first setting.
- Build `offpdf.com` download flows that hide GitHub complexity from end users.

# Roadmap

OffPDF is a free, open-source, offline-first desktop app. The roadmap prioritizes
reliable local processing, clear packaging, and a contributor-friendly project.

## Current release

- **v0.2.2** is the latest public release, distributed through GitHub Releases
  and [offpdf.com](https://offpdf.com).
- The Windows x64 installer bundles qpdf, Poppler, Tesseract, and LibreOffice so
  the supported workflows run offline. The installer is not code-signed yet.
- The Apple Silicon macOS build is signed and notarized. It bundles qpdf,
  Poppler, and Tesseract; LibreOffice remains optional for Office and PDF/A work.
- The app currently includes 22 tools for organizing, converting, optimizing,
  securing, and repairing PDFs.

## Current focus

- Stabilize the visual editor and make exported edits match the on-screen preview.
- Improve automated coverage for complex PDF geometry, rotation, and page boxes.
- Make releases easier to trust and install, starting with Windows code signing.
- Give new contributors smaller, clearly scoped issues with reproducible tests.

## Near term

- Publish explicit checksum files with releases.
- Reduce the Windows package size without weakening offline functionality.
- Add official Intel Mac and Linux packages.
- Improve OCR language selection and optional language-pack guidance.
- Expand editor support for annotations, links, and common form workflows.
- Improve accessibility across keyboard navigation, focus states, and screen-reader labels.

## Known limitations

- Windows may display a SmartScreen warning because the installer is unsigned.
- There are no official Intel Mac or Linux packages yet.
- Office conversion and PDF/A require LibreOffice; it is not bundled on macOS.
- Lossy compression rasterizes pages. Use text-preserving compression when text
  selection matters.
- Complex Office conversions are best effort and may not reproduce every layout.
- Auto-update is intentionally not enabled.

## Later

- Add optional, clearly disclosed update checks behind an offline-first setting.
- Broaden platform packaging and package-manager distribution.
- Add more advanced editing workflows where they can remain reliable and local.

Priorities can change as real bug reports and contributor feedback arrive. See
the [open issues](https://github.com/McanKul/offpdf/issues) for work that is
already scoped.

<p align="center">
  <img src="./docs/assets/offpdf-mark.svg" width="96" height="96" alt="OffPDF logo">
</p>

<h1 align="center">OffPDF</h1>

<p align="center">
  <strong>Private PDF tools that run entirely on your computer.</strong><br>
  23 tools · No uploads · No account · No telemetry · Works offline
</p>

<p align="center">
  <a href="https://github.com/McanKul/offpdf/releases/latest"><img src="https://img.shields.io/github/v/release/McanKul/offpdf?label=release" alt="Latest release"></a>
  <a href="https://github.com/McanKul/offpdf/actions/workflows/ci.yml"><img src="https://github.com/McanKul/offpdf/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI status"></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/McanKul/offpdf" alt="MIT license"></a>
</p>

<p align="center">
  <strong><a href="#downloads">View downloads</a></strong>
  · <a href="https://offpdf.com">Visit offpdf.com</a>
  · <a href="https://github.com/McanKul/offpdf">Star OffPDF on GitHub</a>
  · <a href="./CONTRIBUTING.md">Contribute</a>
</p>

<p align="center">
  <img src="./docs/assets/offpdf-home.png" width="1060" alt="OffPDF desktop app showing its local PDF tools">
</p>

> **Your files never leave your computer.** OffPDF processes documents locally
> and always writes the result to a new file, leaving the original untouched.

OffPDF is a free, MIT-licensed desktop toolbox for everyday PDF work. It uses
local engines such as qpdf, Poppler, Tesseract, and LibreOffice, with no cloud
service in the middle.

## Why OffPDF?

- **Private by design.** No uploads, accounts, analytics, telemetry, or remote API.
- **Useful offline.** Install it once and keep working without a connection.
- **One focused toolbox.** Organize, convert, compress, OCR, protect, and repair PDFs.
- **Open and inspectable.** The app and its privacy model are available here under MIT.

## 23 PDF tools

| Organize | Convert | Optimize & secure |
| --- | --- | --- |
| Merge PDFs | Office / HTML to PDF | Compress PDF |
| Split PDF | PDF to Office | Protect PDF |
| Organize pages | PDF to images | Unlock PDF |
| Page numbers | OCR (make searchable) | Repair PDF |
| Watermark | PDF/A (archive) | Edit metadata |
| Crop pages | PDF to text | |
| Stamp / Sign | | |
| Edit PDF | | |
| Poster / tile print | | |
| N-up / Booklet | | |
| Compare PDFs | | |
| Remove blank pages | | |

## Downloads

| Platform | Package | Status |
| --- | --- | --- |
| Windows x64 | New package not currently published | v0.3 is withheld while Authenticode signing is prepared; older unsigned builds remain in release history |
| macOS 11+ · Apple Silicon | [Download from the latest release](https://github.com/McanKul/offpdf/releases/latest) | Signed and notarized |
| Intel Mac / Linux | [Build from source](#development) | No published package yet |

The Windows installer is large because it includes the local engines needed to
keep its core workflows offline. The macOS build bundles qpdf, Poppler, and
Tesseract; LibreOffice is optional and only needed for Office and PDF/A tools.

All published binaries are available on the
[Releases page](https://github.com/McanKul/offpdf/releases).

See the [code signing policy](./CODE_SIGNING_POLICY.md) for how official release
artifacts are built, approved, and verified.

## Privacy model

- Documents are processed by local binaries on your machine.
- Preview, search, and editing may pass locally generated page data, thumbnails,
  or extracted text to the bundled interface. This data stays inside the app.
- There is no account system, cloud sync, analytics, telemetry, or remote API.
- The production app uses Tauri's local custom protocol and a restrictive CSP.
- Auto-update is not enabled.

See [PRIVACY.md](./PRIVACY.md) for the plain-language privacy statement.

## Development

### Prerequisites

- Node.js 18+
- Rust via `rustup`
- [Tauri v2 platform prerequisites](https://v2.tauri.app/start/prerequisites/)
- `qpdf` for core PDF operations
- Optional: Poppler, Tesseract, and LibreOffice for the tools listed below

```bash
# macOS
brew install qpdf poppler tesseract
brew install --cask libreoffice

# Debian / Ubuntu
sudo apt install qpdf poppler-utils tesseract-ocr libreoffice
```

Install dependencies and run the desktop app:

```bash
npm install
npm run tauri:dev
```

Run the checks used by CI:

```bash
npm run build
npm test
```

Create a desktop bundle:

```bash
npm run tauri:build
```

### Local engines

| Engine | Used for |
| --- | --- |
| `qpdf` | Merge, split, organize, encrypt, decrypt, repair, and lossless optimization |
| Poppler (`pdftoppm`, `pdftotext`) | Previews, image export, comparison, text export, and lossy compression |
| Tesseract | OCR and searchable PDFs |
| LibreOffice | Office conversion and PDF/A export |

<details>
<summary><strong>Project structure</strong></summary>

```text
.
|-- src/                 # React + TypeScript frontend
|   |-- components/      # shared UI, PDF controls, layout, job status
|   |-- features/        # tool-specific screens
|   |-- lib/             # tool registry, validation, Tauri commands
|   |-- routes/          # home, settings, about
|   |-- state/           # Zustand stores
|   `-- styles/
|-- src-tauri/           # Rust backend and Tauri configuration
|   |-- src/
|   |   |-- commands/    # command handlers
|   |   |-- pdf_engine/  # engine resolution and PDF operations
|   |   `-- utils/       # disk, temp, and process helpers
|   `-- tauri.conf.json
`-- public/pdfjs/        # bundled pdf.js assets
```

</details>

## Contributing

Bug reports, feature ideas, documentation improvements, translations, and code
contributions are welcome. Read [CONTRIBUTING.md](./CONTRIBUTING.md) before
opening a larger change, and check the
[open issues](https://github.com/McanKul/offpdf/issues) for current work.

Please report security-sensitive issues privately as described in
[SECURITY.md](./SECURITY.md).

## Project status

OffPDF is under active development. See the [roadmap](./ROADMAP.md) for current
priorities and known platform limitations.

## License

[MIT](./LICENSE)

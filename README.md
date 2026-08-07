# OffPDF

Free, open-source, offline-first desktop PDF tools.

Website: [offpdf.com](https://offpdf.com)

OffPDF is a Tauri v2 desktop app for working with PDFs without uploads, cloud
accounts, telemetry, or a network connection. Files are processed on the user's
machine by local engines such as `qpdf`, `pdftoppm`, Tesseract, and LibreOffice.

The guiding promise is simple:

> Your PDF files never leave your computer.

## Features

OffPDF currently ships 18 tools, grouped the same way as the app UI.

| Category | Tools |
| --- | --- |
| Organize | Merge PDFs, Split PDF, Organize pages, Page numbers, Watermark, Crop pages, Stamp / Sign, Edit PDF, Poster / tile print, Compare PDFs |
| Convert | Office / HTML to PDF, PDF to Office, PDF to images, OCR, PDF/A |
| Optimize and secure | Compress PDF, Protect PDF, Unlock PDF, Repair PDF |

The original file is never overwritten. Every operation writes a new output file.

## Privacy Model

- Files are read from disk and written back to disk locally.
- Only file paths cross the Tauri IPC boundary; PDF bytes are not sent into the
  webview.
- There is no account system, telemetry, cloud sync, analytics, or remote API.
- The production app uses Tauri's local custom protocol and a restrictive CSP.
- Auto-update is not enabled. If it is added later, it should be optional and
  clearly disclosed.

See [PRIVACY.md](./PRIVACY.md) for the user-facing privacy statement.

## Distribution Status

OffPDF is still pre-release. The app is suitable for development and testing,
but broad public distribution should wait until release builds are signed,
notarized where required, and published with checksums.

Recommended public distribution flow:

- Publish binaries through GitHub Releases.
- Point the `offpdf.com` download buttons to the latest signed release assets.
- Keep release notes and checksums in the release page.
- Do not require end users to understand GitHub; the website should expose one
  clear download button per platform.

## Local Engines

OffPDF uses external engines instead of implementing full PDF parsing itself.

| Engine | Used for | Required |
| --- | --- | --- |
| `qpdf` | merge, split, organize, encrypt, decrypt, repair, lossless optimize | Core PDF operations |
| `pdftoppm` / poppler | previews, rendering, image export, visual comparison, lossy compression | Preview/render-heavy tools |
| Tesseract | OCR/searchable PDFs | OCR only |
| LibreOffice | Office conversion and PDF/A export | Office/PDF-A tools |

During development, these tools can be installed on the system `PATH`. Release
builds should bundle the required platform binaries where licensing permits.
Build artifacts and downloaded engine binaries should not be committed to the
repository.

## Prerequisites

- Node.js 18+
- Rust via `rustup`
- Tauri v2 platform prerequisites
- `qpdf` on `PATH` for core PDF operations
- Optional: poppler, Tesseract, LibreOffice for the tools listed above

Platform hints:

```bash
# macOS
brew install qpdf poppler tesseract
brew install --cask libreoffice

# Debian/Ubuntu
sudo apt install qpdf poppler-utils tesseract-ocr libreoffice
```

## Development

Install dependencies:

```bash
npm install
```

Run the desktop app with hot reload:

```bash
npm run tauri:dev
```

Run the frontend only:

```bash
npm run dev
```

Build and test:

```bash
npm run build
npm test
```

Create a desktop bundle:

```bash
npm run tauri:build
```

### Windows Test Builds

Maintainers can create unsigned Windows test installers from GitHub Actions:

1. Open the **Windows Build** workflow.
2. Run it manually with the default engine versions, or pass pinned qpdf/poppler versions.
3. Download the `offpdf-windows-*` artifact.

The workflow downloads the official qpdf Windows `msvc64` zip and the
poppler-windows release zip. It copies `qpdf.exe`, `pdftoppm.exe`,
`pdftotext.exe`, their DLLs, and poppler data into Tauri resources, then runs
`npm run tauri:build`.
Those binaries are bundled into the installer but are not committed to git.
The NSIS installer targets the current user so early testers do not need
administrator privileges just to install OffPDF.

On macOS, a headless shell may fail during the `.dmg` step because the Tauri DMG
script needs a real GUI session. To build only the `.app` bundle:

```bash
npm run tauri build -- --bundles app
```

## Project Structure

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
|   |   |-- commands/    # Tauri command handlers
|   |   |-- pdf_engine/  # engine resolution and PDF operations
|   |   |-- utils/       # disk, temp, process helpers
|   |   `-- lib.rs
|   |-- binaries/        # optional bundled engines, not committed
|   `-- tauri.conf.json
`-- public/pdfjs/        # local pdf.js assets
```

## Release Notes For Maintainers

- Keep the app fully usable offline.
- Keep build outputs, signing keys, certificates, and downloaded engine binaries
  out of git.
- Prefer local subprocess execution with argument arrays. Avoid shell string
  interpolation for user-provided file paths.
- Review licenses before adding PDF engines or bundling third-party binaries.
- Generated icons live in `src-tauri/icons/`; regenerate them from a 1024x1024
  source image with:

```bash
npm run tauri icon path/to/source-1024.png
```

## Contributing

Issues and pull requests are welcome. Please read
[CONTRIBUTING.md](./CONTRIBUTING.md) before opening larger changes.

Security-sensitive issues should follow [SECURITY.md](./SECURITY.md).

## Roadmap

See [ROADMAP.md](./ROADMAP.md).

## License

MIT - see [LICENSE](./LICENSE).

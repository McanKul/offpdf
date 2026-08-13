# Privacy

**Your PDF files never leave your computer.**

OffPDF is built to be private by design. This is the plain-language summary of
what it does and does not do with your data.

## What OffPDF does NOT do

OffPDF does **not** upload, send, sync, or transmit any of the following,
anywhere:

- Your PDF files or their contents
- File names or file paths
- PDF metadata
- Page thumbnails or previews
- Usage analytics or telemetry

The application works **fully offline**. The backend never makes a network
request, there is no account to sign in to, and there is no cloud component.
The app's Content-Security-Policy restricts everything to local origins only.

## How your files are processed

PDF operations run entirely on your machine using local engines such as qpdf,
Poppler, Tesseract, and LibreOffice. Core operations read from and write to disk
without uploading anything.

For previews, search, and editing, the bundled interface may receive locally
generated thumbnails, extracted text, or one extracted PDF page. This data stays
inside the app and is never transmitted to OffPDF or any third party.

## What IS stored locally

OffPDF stores a small amount of preference and convenience data **on your
computer only**. This **never includes PDF content**:

- **Recent-jobs metadata** — e.g. which tool was run, the result status, and
  local output paths, so you can see and reopen recent results.
- **Last output folder** — so the next save defaults to a sensible location.
- **Theme** — your light/dark preference.

None of this is transmitted anywhere.

## Temporary files

Some operations create temporary working files on disk. These are cleaned up
automatically when a job finishes. You can also clear them manually at any time
from **Settings → Clear temporary files**, which frees the space used by
OffPDF's temp folder.

## Questions

OffPDF is local-first and offline-only by design. If a future version ever
introduces an optional networked feature, it will be clearly disclosed and
disabled by default.

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

PDF operations run entirely on your machine by spawning a local `qpdf` binary.
Files are read and written directly on disk. Only file *paths* are passed between
the user interface and the backend — your file bytes are never sent across that
boundary or loaded into the UI.

## What IS stored locally

OffPDF stores a small amount of preference and convenience data **on your
computer only**. This **never includes PDF content**:

- **Recent-jobs metadata** — e.g. which tool was run and the result status, so
  you can see your recent activity.
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

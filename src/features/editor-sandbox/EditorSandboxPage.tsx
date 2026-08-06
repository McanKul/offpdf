/**
 * Dev sandbox for the issue #6 visual editor canvas.
 * Open from the sidebar (crop icon) or #/dev/editor-canvas inside the desktop app.
 */
import { useState } from "react";
import { WorkspaceFilePicker } from "@/components/pdf";
import { PdfEditorCanvas } from "@/components/pdf/editor";
import { Alert } from "@/components/ui/Alert";
import { useWorkspace } from "@/state/workspaceStore";
import type { EditDocument } from "@/lib/editor";
import { isTauriRuntime } from "@/lib/tauriEnv";

export function EditorSandboxPage() {
  const files = useWorkspace((s) => s.files);
  const first = files.find((f) => f.isValidPdf) ?? files[0];
  const [page, setPage] = useState(1);
  const [doc, setDoc] = useState<EditDocument | null>(null);
  const inTauri = isTauriRuntime();

  const pageCount = first?.pageCount && first.pageCount > 0 ? first.pageCount : 1;

  return (
    <div className="col" style={{ gap: 16, padding: "8px 0 24px" }}>
      <div>
        <h1 style={{ fontSize: 20 }}>Editor canvas (dev)</h1>
        <p className="muted" style={{ marginTop: 6, maxWidth: 640 }}>
          Sandbox for issue #6 — the reusable page editor canvas. MVP drafts are{" "}
          <strong>rectangles only</strong> (select, move, resize, undo). Text, images,
          freehand, object rotation, and saving into a PDF come in later issues (#7–#8).
          Nothing is written into the PDF here.
        </p>
      </div>

      {!inTauri && (
        <Alert variant="warning">
          You are in a <strong>browser-only</strong> session. OffPDF needs the desktop app for
          file pick, drop, and page preview. Stop this tab and run{" "}
          <code className="mono">npm run tauri:dev</code>, then open this page from the sidebar
          (crop icon near Settings).
        </Alert>
      )}

      {inTauri && (
        <Alert variant="info">
          Add a PDF below (click the drop zone or drag a file onto the OffPDF window), then draw
          rectangles on the page.
        </Alert>
      )}

      <WorkspaceFilePicker selectable={false} />

      {inTauri && !first && (
        <Alert variant="info">Add a PDF above to try the visual editor canvas.</Alert>
      )}

      {first && !first.isValidPdf && (
        <Alert variant="danger">The selected file is not a valid PDF.</Alert>
      )}

      {first?.isValidPdf && inTauri && (
        <>
          <PdfEditorCanvas
            sourcePath={first.path}
            pageNumber={page}
            pageCount={pageCount}
            onPageChange={setPage}
            onChange={setDoc}
          />
          {doc && (
            <details className="pdf-editor__debug">
              <summary className="muted" style={{ cursor: "pointer", fontSize: 12.5 }}>
                Edit model JSON ({doc.objects.length} object
                {doc.objects.length === 1 ? "" : "s"})
              </summary>
              <pre className="pdf-editor__debug-pre mono">{JSON.stringify(doc, null, 2)}</pre>
            </details>
          )}
        </>
      )}
    </div>
  );
}

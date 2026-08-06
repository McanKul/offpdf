import { useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Icon } from "@/components/ui/Icon";
import { pickPdfFile, pickPdfFiles } from "@/lib/tauriCommands";
import { toAppError } from "@/lib/types";
import { useToast } from "@/components/ui/Toast";
import { SUPPORTED_RE } from "@/lib/fileTypes";
import { isTauriRuntime } from "@/lib/tauriEnv";

/**
 * Click to open the native file dialog, or drag-and-drop PDFs onto it.
 * Only file *paths* are produced; bytes never enter the webview.
 *
 * Requires the Tauri desktop app (`npm run tauri:dev`). Browser-only Vite does
 * not expose OS paths or the native picker.
 */
export function Dropzone({
  multiple,
  onFiles,
  title,
  hint,
  compact = false,
}: {
  multiple: boolean;
  onFiles: (paths: string[]) => void;
  title?: string;
  hint?: string;
  compact?: boolean;
}) {
  const [dragging, setDragging] = useState(false);
  const { toast } = useToast();
  const inTauri = isTauriRuntime();

  // Native OS drag-and-drop (Tauri delivers real file paths).
  useEffect(() => {
    if (!inTauri) return;
    let unlisten: (() => void) | undefined;
    let active = true;
    (async () => {
      try {
        const fn = await getCurrentWebview().onDragDropEvent((event) => {
          const p = event.payload;
          if (p.type === "enter" || p.type === "over") setDragging(true);
          else if (p.type === "leave") setDragging(false);
          else if (p.type === "drop") {
            setDragging(false);
            const paths = p.paths.filter((x) => SUPPORTED_RE.test(x));
            if (paths.length > 0) onFiles(multiple ? paths : paths.slice(-1));
            else
              toast({
                title: "Only PDF, image, or Office files are supported",
                variant: "error",
              });
          }
        });
        if (active) unlisten = fn;
        else fn();
      } catch {
        // Not in a Tauri webview, or drag-drop unavailable — click still works in-app.
      }
    })();
    return () => {
      active = false;
      unlisten?.();
    };
  }, [inTauri, multiple, onFiles, toast]);

  const openDialog = async () => {
    if (!inTauri) {
      toast({
        title: "Use the desktop app",
        description:
          "File pick and drop need OffPDF’s desktop shell. Run: npm run tauri:dev — then open Editor canvas from the sidebar. A normal browser tab cannot access local file paths.",
        variant: "error",
        duration: 8000,
      });
      return;
    }
    try {
      if (multiple) {
        const paths = await pickPdfFiles();
        if (paths.length) onFiles(paths);
      } else {
        const path = await pickPdfFile();
        if (path) onFiles([path]);
      }
    } catch (e) {
      toast({
        title: "Could not open file picker",
        description: toAppError(e).message,
        variant: "error",
      });
    }
  };

  if (compact) {
    return (
      <div
        className={`dropzone dropzone--compact ${dragging ? "is-dragging" : ""}`}
        onClick={openDialog}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") openDialog();
        }}
      >
        <Icon name="plus" size={18} className="dropzone__icon" />
        <div className="dropzone__title">{title ?? "Add PDFs"}</div>
      </div>
    );
  }

  return (
    <div
      className={`dropzone ${dragging ? "is-dragging" : ""}`}
      onClick={openDialog}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") openDialog();
      }}
    >
      <Icon name="upload" size={30} className="dropzone__icon" />
      <div className="dropzone__title">
        {title ?? "Drop PDFs, images or Office files here, or click to browse"}
      </div>
      <div className="dropzone__hint">
        {inTauri
          ? (hint ??
            "PDF, images (PNG/JPG…), Word/Excel/PowerPoint — processed locally, never uploaded.")
          : "Desktop app required — run npm run tauri:dev (browser cannot open local files)."}
      </div>
    </div>
  );
}

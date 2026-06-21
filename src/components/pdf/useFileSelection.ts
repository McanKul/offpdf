/**
 * Manages a list of selected PDFs as `FileInfo` (path + size + page count).
 * Only paths are sent to the backend; we fetch metadata via `get_file_info`.
 * Invalid / non-PDF files are surfaced as a toast and skipped.
 */
import { useCallback, useState } from "react";
import { getFileInfo } from "@/lib/tauriCommands";
import { toAppError, type FileInfo } from "@/lib/types";
import { useToast } from "@/components/ui/Toast";

export interface FileSelection {
  files: FileInfo[];
  loading: boolean;
  /** Add paths (from dialog or drag-drop). Replaces selection in single mode. */
  addPaths: (paths: string[]) => Promise<void>;
  removeAt: (index: number) => void;
  reorder: (from: number, to: number) => void;
  clear: () => void;
}

export function useFileSelection(multiple: boolean): FileSelection {
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const { toast } = useToast();

  const addPaths = useCallback(
    async (paths: string[]) => {
      const pdfPaths = paths.filter((p) => /\.pdf$/i.test(p));
      if (pdfPaths.length === 0) {
        if (paths.length > 0) {
          toast({ title: "Only PDF files are supported", variant: "error" });
        }
        return;
      }

      setLoading(true);
      try {
        const infos = await Promise.all(
          pdfPaths.map(async (path) => {
            try {
              return await getFileInfo(path);
            } catch (e) {
              toast({
                title: "Could not read file",
                description: toAppError(e).message,
                variant: "error",
              });
              return null;
            }
          }),
        );

        const valid = infos.filter((i): i is FileInfo => i !== null);
        const invalid = valid.filter((i) => !i.isValidPdf);
        if (invalid.length > 0) {
          toast({
            title: invalid.length === 1 ? "That file is not a valid PDF" : "Some files are not valid PDFs",
            description: invalid.map((i) => i.name).join(", "),
            variant: "error",
          });
        }
        const usable = valid.filter((i) => i.isValidPdf);

        setFiles((prev) => {
          if (!multiple) return usable.slice(-1);
          // Dedupe by path, keep existing order, append new.
          const seen = new Set(prev.map((f) => f.path));
          return [...prev, ...usable.filter((f) => !seen.has(f.path))];
        });
      } finally {
        setLoading(false);
      }
    },
    [multiple, toast],
  );

  const removeAt = useCallback((index: number) => {
    setFiles((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const reorder = useCallback((from: number, to: number) => {
    setFiles((prev) => {
      if (from === to || from < 0 || to < 0 || from >= prev.length || to >= prev.length) {
        return prev;
      }
      const next = [...prev];
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved);
      return next;
    });
  }, []);

  const clear = useCallback(() => setFiles([]), []);

  return { files, loading, addPaths, removeAt, reorder, clear };
}

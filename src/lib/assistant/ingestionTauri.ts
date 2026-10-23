/**
 * Tauri adapter for private-assistant ingestion.
 *
 * Only paths and text cross IPC. OCR runs in the native backend, which removes
 * its temporary searchable PDF before returning, including on cancellation.
 */

import {
  assistantOcrText,
  assistantPdfText,
  cancelJob,
  getFileInfo,
  newJobId,
  onJobUpdate,
} from "@/lib/tauriCommands";
import type { IngestionRuntime } from "./ingestion";

export function createTauriIngestionRuntime(): IngestionRuntime {
  return {
    inspect: async (path) => {
      const info = await getFileInfo(path);
      return {
        sizeBytes: info.sizeBytes,
        pageCount: info.pageCount ?? null,
        isValidPdf: info.isValidPdf,
      };
    },

    extractText: (path) => assistantPdfText(path),

    ocrText: async ({ path, pages, language, signal, onProgress }) => {
      if (signal.aborted) {
        throw new DOMException("Ingestion cancelled", "AbortError");
      }

      const jobId = newJobId();
      const unlisten = await onJobUpdate((update) => {
        if (typeof update.percent === "number") {
          onProgress?.(update.percent);
        }
      }, jobId);
      const onAbort = () => {
        void cancelJob(jobId).catch(() => undefined);
      };
      signal.addEventListener("abort", onAbort, { once: true });

      try {
        // Cancellation may have arrived while the event listener was being
        // installed; do not start a job that the user already cancelled.
        if (signal.aborted) {
          throw new DOMException("Ingestion cancelled", "AbortError");
        }
        const text = await assistantOcrText(jobId, path, [...pages], language);
        if (signal.aborted) {
          throw new DOMException("Ingestion cancelled", "AbortError");
        }
        return text;
      } finally {
        signal.removeEventListener("abort", onAbort);
        unlisten();
      }
    },
  };
}

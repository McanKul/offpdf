import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauriCommands", () => ({
  getFileInfo: async () => ({
    path: "/tmp/offpdf-edit-out.pdf",
    name: "offpdf-edit-out.pdf",
    sizeBytes: 1,
    pageCount: 1,
    isValidPdf: true,
  }),
  openPath: async () => {},
  imageToPdf: async () => "/tmp/x.pdf",
  officeToPdf: async () => "/tmp/x.pdf",
}));

import { ToastProvider } from "@/components/ui/Toast";
import { JobStatus } from "./JobStatus";
import type { JobController } from "./useJob";

const QPDF_WARNING = "WARNING: qpdf --check: file is not linearized";

function completedJob(message: string | null): JobController {
  return {
    state: "completed",
    update: {
      jobId: "job-34",
      state: "completed",
      step: "Done",
      message,
    },
    result: {
      jobId: "job-34",
      outputPaths: ["/tmp/offpdf-edit-out.pdf"],
      status: "ok",
    },
    error: null,
    meta: null,
    isBusy: false,
    run: async () => {},
    cancel: () => {},
    reset: () => {},
  };
}

describe("JobStatus completed card", () => {
  it("shows job.update.message on the Done card", () => {
    const markup = renderToStaticMarkup(
      <ToastProvider>
        <JobStatus job={completedJob(QPDF_WARNING)} />
      </ToastProvider>,
    );
    expect(markup).toContain("Done");
    expect(markup).toContain(QPDF_WARNING);
  });
});

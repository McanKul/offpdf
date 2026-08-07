import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { useToast } from "@/components/ui/Toast";
import {
  WorkspaceFilePicker,
  OutputFolderPicker,
  LargeFileWarning,
  useCombinedDoc,
  buildGroups,
  PdfEditorCanvas,
} from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { editPdfOverlays } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";
import { createEmptyDocument, toExportDocument, type EditDocument } from "@/lib/editor";
import { isTauriRuntime } from "@/lib/tauriEnv";
import { Alert } from "@/components/ui/Alert";

const tool = getTool("editPdf");

export function EditPdfPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();
  const inTauri = isTauriRuntime();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("edited.pdf");
  const [pageIndex, setPageIndex] = useState(0);
  const [doc, setDoc] = useState<EditDocument>(createEmptyDocument());

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-edited.pdf`);
  }, [first?.path]);

  useEffect(() => {
    if (pageIndex >= refs.length) setPageIndex(Math.max(0, refs.length - 1));
  }, [refs.length, pageIndex]);

  const resetKey = refs.map((r) => r.key).join("|");
  const current = refs[pageIndex];

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (doc.objects.length === 0) return toast({ title: "Add something to the page first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (files.some((f) => f.path === outputPath)) {
      return toast({ title: "Choose a new file name", description: "The original file is never overwritten.", variant: "error" });
    }
    if (!(await disk.ensure(folder, estimateRequiredBytes("editPdf", files.map((f) => f.sizeBytes))))) return;

    await job.run(
      (id) => editPdfOverlays(id, outputPath, buildGroups(refs), toExportDocument(doc)),
      { tool: "editPdf", label: `Edit PDF · ${doc.objects.length} object${doc.objects.length === 1 ? "" : "s"}` },
    );
  };

  const canStart = refs.length > 0 && !!folder && doc.objects.length > 0 && !job.isBusy && inTauri;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Documents">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      {!inTauri && (
        <Alert variant="warning">Open the desktop app to edit pages and save a new PDF.</Alert>
      )}

      {current && inTauri && current && (
        <ToolSection label="Edit" sublabel="Existing page content stays as-is. Draw on top, then save a new file.">
          <PdfEditorCanvas
            sourcePath={current.path}
            sourcePage={current.page}
            pageIndex={pageIndex}
            pageCount={refs.length}
            resetKey={resetKey}
            onPageChange={setPageIndex}
            onChange={setDoc}
          />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="edited.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="fileText" size={18} />}>
          Save PDF
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

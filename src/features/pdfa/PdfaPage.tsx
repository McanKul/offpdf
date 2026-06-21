import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import {
  WorkspaceFilePicker,
  CombinedPreview,
  OutputFolderPicker,
  LargeFileWarning,
  useCombinedDoc,
  buildGroups,
} from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { pdfaPdf, officeAvailable } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("pdfa");

export function PdfaPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("archive.pdf");
  const [available, setAvailable] = useState(true);

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-pdfa.pdf`);
  }, [first?.path]);
  useEffect(() => {
    let on = true;
    officeAvailable().then((v) => on && setAvailable(v)).catch(() => {});
    return () => {
      on = false;
    };
  }, []);

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => pdfaPdf(id, outputPath, buildGroups(refs)), {
      tool: "pdfa",
      label: `PDF/A ${refs.length} pages`,
    });
  };

  const canStart = refs.length > 0 && !!folder && available && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      {!available && (
        <Alert variant="warning" title="LibreOffice needed">
          PDF/A conversion uses LibreOffice, which wasn’t found. Install it (macOS:{" "}
          <span className="mono">brew install --cask libreoffice</span>), then reopen this tool.
        </Alert>
      )}

      <ToolSection label="Documents">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      <ToolSection label="About PDF/A">
        <Alert variant="info">
          PDF/A-2b is the ISO standard for long-term archiving: fully self-contained with embedded
          fonts. Conversion goes through LibreOffice, so complex layouts may shift slightly.
        </Alert>
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="archive.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="badge" size={18} />}>
          Convert to PDF/A
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

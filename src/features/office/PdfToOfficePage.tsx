import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Select } from "@/components/ui/Select";
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
import { pdfToOffice, officeAvailable } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes } from "@/lib/validation";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("pdfToOffice");

export function PdfToOfficePage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  // "xlsx" is intentionally not offered: LibreOffice has no PDF import filter
  // for Calc, so PDF → Excel can never succeed.
  const [format, setFormat] = useState<"docx" | "pptx">("docx");
  const [available, setAvailable] = useState(true);

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
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;
    await job.run((id) => pdfToOffice(id, folder, buildGroups(refs), format), {
      tool: "pdfToOffice",
      label: `PDF → ${format.toUpperCase()}`,
    });
  };

  const canStart = refs.length > 0 && !!folder && available && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      {!available && (
        <Alert variant="warning" title="LibreOffice needed">
          This conversion uses LibreOffice, which wasn’t found. Install it from libreoffice.org
          (macOS: <span className="mono">brew install --cask libreoffice</span>), then reopen this tool.
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

      <ToolSection label="Convert to">
        <Select
          label="Format"
          value={format}
          onChange={(v) => setFormat(v as "docx" | "pptx")}
          options={[
            { value: "docx", label: "Word (.docx)" },
            { value: "pptx", label: "PowerPoint (.pptx)" },
          ]}
        />
        <div className="mt">
          <Alert variant="info">
            Best effort: text is recovered well, but complex page layouts, columns and graphics may not
            be reproduced exactly. For a faithful copy, keep the PDF. Excel output isn&apos;t supported —
            convert to Word and copy tables from there.
          </Alert>
        </div>
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <OutputFolderPicker value={folder} onChange={setFolder} />
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="fileText" size={18} />}>
          Convert to {format.toUpperCase()}
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

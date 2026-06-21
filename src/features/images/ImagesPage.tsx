import { useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Icon } from "@/components/ui/Icon";
import { Select } from "@/components/ui/Select";
import { useToast } from "@/components/ui/Toast";
import {
  WorkspaceFilePicker,
  CombinedPreview,
  OutputFolderPicker,
  LargeFileWarning,
  useCombinedDoc,
  buildPicks,
} from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { pdfToImages } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes } from "@/lib/validation";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("images");

const RES = [
  { value: "96", label: "Screen (96 DPI)" },
  { value: "150", label: "Print (150 DPI)" },
  { value: "300", label: "High (300 DPI)" },
];

export function ImagesPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [format, setFormat] = useState<"png" | "jpg">("png");
  const [dpi, setDpi] = useState("150");

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    if (!(await disk.ensure(folder, estimateRequiredBytes("split", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => pdfToImages(id, folder, buildPicks(refs), format, Number(dpi)), {
      tool: "images",
      label: `${refs.length} pages → ${format.toUpperCase()}`,
    });
  };

  const canStart = refs.length > 0 && !!folder && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Documents" sublabel="Every page becomes one image file.">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      <ToolSection label="Image options">
        <div className="row">
          <Select
            label="Format"
            value={format}
            onChange={(v) => setFormat(v as "png" | "jpg")}
            options={[
              { value: "png", label: "PNG (lossless, larger)" },
              { value: "jpg", label: "JPG (smaller)" },
            ]}
          />
          <Select label="Resolution" value={dpi} onChange={setDpi} options={RES} />
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
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="image" size={18} />}>
          Export images
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

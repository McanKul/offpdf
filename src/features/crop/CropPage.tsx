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
import { cropPdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("crop");

export function CropPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("cropped.pdf");
  const [left, setLeft] = useState("5");
  const [top, setTop] = useState("5");
  const [right, setRight] = useState("5");
  const [bottom, setBottom] = useState("5");

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-cropped.pdf`);
  }, [first?.path]);

  const nums = [left, top, right, bottom].map(Number);
  const valid = nums.every((v) => Number.isFinite(v) && v >= 0 && v < 50);

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!valid) return toast({ title: "Margins must be between 0 and 49 (%)", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => cropPdf(id, outputPath, buildGroups(refs), nums[0], nums[1], nums[2], nums[3]), {
      tool: "crop",
      label: `Crop ${refs.length} pages`,
    });
  };

  const canStart = refs.length > 0 && !!folder && valid && !job.isBusy;

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

      <ToolSection label="Trim (% of each edge)">
        <div className="row">
          <Input label="Left" type="number" min={0} max={49} value={left} onChange={(e) => setLeft(e.target.value)} />
          <Input label="Top" type="number" min={0} max={49} value={top} onChange={(e) => setTop(e.target.value)} />
          <Input label="Right" type="number" min={0} max={49} value={right} onChange={(e) => setRight(e.target.value)} />
          <Input label="Bottom" type="number" min={0} max={49} value={bottom} onChange={(e) => setBottom(e.target.value)} />
        </div>
        <div className="mt">
          <Alert variant="info">Lossless — only the visible page area changes; text and vectors are kept.</Alert>
        </div>
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview (before crop)">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="cropped.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="crop" size={18} />}>
          Crop pages
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

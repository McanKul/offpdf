import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Icon } from "@/components/ui/Icon";
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
import { watermarkPdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("watermark");

export function WatermarkPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("watermarked.pdf");
  const [text, setText] = useState("DRAFT");
  const [opacity, setOpacity] = useState("0.3");

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-watermarked.pdf`);
  }, [first?.path]);

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!text.trim()) return toast({ title: "Enter the watermark text", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => watermarkPdf(id, outputPath, buildGroups(refs), text, Number(opacity)), {
      tool: "watermark",
      label: `Watermark ${refs.length} pages`,
    });
  };

  const canStart = refs.length > 0 && !!folder && text.trim().length > 0 && !job.isBusy;

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

      <ToolSection label="Watermark">
        <div className="row">
          <Input label="Text" value={text} onChange={(e) => setText(e.target.value)} placeholder="e.g. CONFIDENTIAL" />
          <Select
            label="Opacity"
            value={opacity}
            onChange={setOpacity}
            options={[
              { value: "0.15", label: "Light" },
              { value: "0.3", label: "Medium" },
              { value: "0.5", label: "Strong" },
            ]}
          />
        </div>
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview (watermark added on output)">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="watermarked.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="droplet" size={18} />}>
          Add watermark
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

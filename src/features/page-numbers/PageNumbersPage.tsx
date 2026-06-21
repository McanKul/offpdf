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
import { addPageNumbers } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("pageNumbers");

const POSITIONS = [
  { value: "bottom-center", label: "Bottom center" },
  { value: "bottom-right", label: "Bottom right" },
  { value: "bottom-left", label: "Bottom left" },
  { value: "top-center", label: "Top center" },
  { value: "top-right", label: "Top right" },
  { value: "top-left", label: "Top left" },
];

export function PageNumbersPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("numbered.pdf");
  const [position, setPosition] = useState("bottom-center");
  const [start, setStart] = useState("1");

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-numbered.pdf`);
  }, [first?.path]);

  const run = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const startNum = Number(start);
    if (!Number.isInteger(startNum)) return toast({ title: "Start number must be a whole number", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => addPageNumbers(id, outputPath, buildGroups(refs), position, startNum), {
      tool: "pageNumbers",
      label: `Number ${refs.length} pages`,
    });
  };

  const canStart = refs.length > 0 && !!folder && !job.isBusy;

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

      <ToolSection label="Numbering">
        <div className="row">
          <Select label="Position" value={position} onChange={setPosition} options={POSITIONS} />
          <Input label="Start at" type="number" value={start} onChange={(e) => setStart(e.target.value)} hint="First page's number." />
        </div>
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="numbered.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={run} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="hash" size={18} />}>
          Add page numbers
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

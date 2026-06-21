import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { useToast } from "@/components/ui/Toast";
import { WorkspaceFilePicker, PageEditor, OutputFolderPicker, LargeFileWarning } from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { editPdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import type { PageGroup, RotateGroup } from "@/lib/types";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("reorder");

export function ReorderPage() {
  const files = useWorkspace((s) => s.files);
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("edited.pdf");
  const [plan, setPlan] = useState<{ groups: PageGroup[]; rotations: RotateGroup[]; count: number }>({
    groups: [],
    rotations: [],
    count: 0,
  });

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-edited.pdf`);
  }, [first?.path]);

  const start = async () => {
    if (plan.count === 0) return toast({ title: "Keep at least one page", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });

    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("reorder", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => editPdf(id, outputPath, plan.groups, plan.rotations), {
      tool: "reorder",
      label: `Edit ${plan.count} pages`,
    });
  };

  const canStart = plan.count > 0 && !!folder && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Documents" sublabel="All loaded files appear below as one document.">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      <ToolSection
        label="Edit pages"
        sublabel="Drag to reorder · ⟳ rotate · ✕ delete · Undo to bring pages back."
      >
        <PageEditor onChange={(groups, rotations, count) => setPlan({ groups, rotations, count })} />
      </ToolSection>

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="edited.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="reorder" size={18} />}>
          Save PDF
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

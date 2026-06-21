import { useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { useToast } from "@/components/ui/Toast";
import { Dropzone, SortableFileList, OutputFolderPicker, LargeFileWarning } from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { mergePdfs } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("merge");

export function MergePage() {
  const files = useWorkspace((s) => s.files);
  const addPaths = useWorkspace((s) => s.addPaths);
  const reorder = useWorkspace((s) => s.reorder);
  const removeAt = useWorkspace((s) => s.removeAt);

  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("merged.pdf");

  const add = async (paths: string[]) => {
    const r = await addPaths(paths);
    if (r.notPdf) toast({ title: "Only PDF, image, or Office files are supported", variant: "error" });
    if (r.errors.length)
      toast({ title: "Some files could not be added", description: r.errors.join(" · "), variant: "error" });
    if (r.invalid.length)
      toast({ title: "Some files are not valid PDFs", description: r.invalid.join(", "), variant: "error" });
  };

  const start = async () => {
    const paths = files.map((f) => f.path);
    if (paths.length < 2) {
      toast({ title: "Add at least two PDFs to merge", variant: "error" });
      return;
    }
    if (!folder) {
      toast({ title: "Choose an output folder", variant: "error" });
      return;
    }
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) {
      toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
      return;
    }

    const outputPath = joinPath(folder, nameRes.value);
    const required = estimateRequiredBytes("merge", files.map((f) => f.sizeBytes));
    if (!(await disk.ensure(folder, required))) return;

    await job.run((id) => mergePdfs(id, paths, outputPath), {
      tool: "merge",
      label: `Merge ${paths.length} files`,
    });
  };

  const canStart = files.length >= 2 && !!folder && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Files to merge" sublabel="Drag the rows to set the merge order.">
        {files.length === 0 ? (
          <Dropzone multiple onFiles={add} />
        ) : (
          <>
            <SortableFileList files={files} onReorder={reorder} onRemove={removeAt} />
            <div className="mt">
              <Dropzone multiple onFiles={add} compact />
            </div>
            <div className="mt">
              <LargeFileWarning files={files} />
            </div>
          </>
        )}
      </ToolSection>

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="merged.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button
          variant="primary"
          size="lg"
          onClick={start}
          disabled={!canStart}
          loading={job.isBusy}
          leftIcon={<Icon name="merge" size={18} />}
        >
          Merge PDFs
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}

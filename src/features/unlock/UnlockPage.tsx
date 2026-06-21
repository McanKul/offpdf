import { useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import { Dropzone, OutputFolderPicker } from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { unlockPdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { isPdfPath } from "@/lib/fileTypes";
import { validateOutputName, joinPath } from "@/lib/validation";
import { basename, stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";

const tool = getTool("unlock");

export function UnlockPage() {
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();
  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);

  const [path, setPath] = useState<string | null>(null);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("unlocked.pdf");
  const [pw, setPw] = useState("");

  // Encrypted PDFs can't load into the normal workspace, so pick directly.
  const onFiles = (paths: string[]) => {
    const pdf = paths.filter(isPdfPath).pop();
    if (!pdf) return toast({ title: "Pick a PDF file", variant: "error" });
    setPath(pdf);
    setName(`${stripExt(basename(pdf))}-unlocked.pdf`);
  };

  const start = async () => {
    if (!path) return toast({ title: "Select the protected PDF", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    if (!pw) return toast({ title: "Enter the password", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, 0))) return;

    await job.run((id) => unlockPdf(id, path, outputPath, pw), {
      tool: "unlock",
      label: `Unlock ${basename(path)}`,
    });
  };

  const canStart = !!path && !!folder && pw.length > 0 && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Protected PDF" sublabel="Pick the password-protected file and enter its password.">
        <Dropzone multiple={false} onFiles={onFiles} title="Drop the protected PDF, or click to browse" hint="The file stays on your computer." />
        {path && (
          <div className="file-list mt">
            <div className="file-row">
              <div className="file-row__icon">
                <Icon name="lock" size={18} />
              </div>
              <div className="grow">
                <div className="file-row__name truncate" title={path}>
                  {basename(path)}
                </div>
              </div>
              <button className="btn btn--ghost btn--sm" onClick={() => setPath(null)} title="Remove">
                <Icon name="x" size={16} />
              </button>
            </div>
          </div>
        )}
      </ToolSection>

      <ToolSection label="Password">
        <Input label="Current password" type="password" value={pw} onChange={(e) => setPw(e.target.value)} placeholder="The password needed to open it" />
        <div className="mt">
          <Alert variant="info">Only works if you know the password — this removes it, it doesn’t crack it.</Alert>
        </div>
      </ToolSection>

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="unlocked.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="unlock" size={18} />}>
          Remove password
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
